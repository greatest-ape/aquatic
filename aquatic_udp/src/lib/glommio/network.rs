use std::cell::RefCell;
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use futures_lite::{Stream, StreamExt};
use glommio::enclose;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::channels::local_channel::{new_unbounded, LocalSender};
use glommio::net::UdpSocket;
use glommio::prelude::*;
use glommio::timer::TimerActionRepeat;
use rand::prelude::{Rng, SeedableRng, StdRng};

use aquatic_udp_protocol::{IpVersion, Request, Response};

use super::common::update_access_list;

use crate::common::network::ConnectionMap;
use crate::common::*;
use crate::config::Config;

pub async fn run_socket_worker(
    config: Config,
    request_mesh_builder: MeshBuilder<(usize, AnnounceRequest, SocketAddr), Partial>,
    response_mesh_builder: MeshBuilder<(AnnounceResponse, SocketAddr), Partial>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let (local_sender, local_receiver) = new_unbounded();

    let mut socket = UdpSocket::bind(config.network.address).unwrap();

    let recv_buffer_size = config.network.socket_recv_buffer_size;

    if recv_buffer_size != 0 {
        socket.set_buffer_size(recv_buffer_size);
    }

    let socket = Rc::new(socket);

    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let (request_senders, _) = request_mesh_builder.join(Role::Producer).await.unwrap();

    let (_, mut response_receivers) = response_mesh_builder.join(Role::Consumer).await.unwrap();

    let response_consumer_index = response_receivers.consumer_id().unwrap();

    spawn_local(read_requests(
        config.clone(),
        request_senders,
        response_consumer_index,
        local_sender,
        socket.clone(),
    ))
    .detach();

    for (_, receiver) in response_receivers.streams().into_iter() {
        spawn_local(send_responses(
            socket.clone(),
            receiver.map(|(response, addr)| (response.into(), addr)),
        ))
        .detach();
    }

    send_responses(socket, local_receiver.stream()).await;
}

async fn read_requests(
    config: Config,
    request_senders: Senders<(usize, AnnounceRequest, SocketAddr)>,
    response_consumer_index: usize,
    local_sender: LocalSender<(Response, SocketAddr)>,
    socket: Rc<UdpSocket>,
) {
    let mut rng = StdRng::from_entropy();

    let access_list_mode = config.access_list.mode;

    let max_connection_age = config.cleaning.max_connection_age;
    let connection_valid_until = Rc::new(RefCell::new(ValidUntil::new(max_connection_age)));
    let access_list = Rc::new(RefCell::new(AccessList::default()));
    let connections = Rc::new(RefCell::new(ConnectionMap::default()));

    // Periodically update connection_valid_until
    TimerActionRepeat::repeat(enclose!((connection_valid_until) move || {
        enclose!((connection_valid_until) move || async move {
            *connection_valid_until.borrow_mut() = ValidUntil::new(max_connection_age);

            Some(Duration::from_secs(1))
        })()
    }));

    // Periodically update access list
    TimerActionRepeat::repeat(enclose!((config, access_list) move || {
        enclose!((config, access_list) move || async move {
            update_access_list(config.clone(), access_list.clone()).await;

            Some(Duration::from_secs(config.cleaning.interval))
        })()
    }));

    // Periodically clean connections
    TimerActionRepeat::repeat(enclose!((config, connections) move || {
        enclose!((config, connections) move || async move {
            connections.borrow_mut().clean();

            Some(Duration::from_secs(config.cleaning.interval))
        })()
    }));

    let mut buf = [0u8; MAX_PACKET_SIZE];

    loop {
        match socket.recv_from(&mut buf).await {
            Ok((amt, src)) => {
                let request = Request::from_bytes(&buf[..amt], config.protocol.max_scrape_torrents);

                ::log::debug!("read request: {:?}", request);

                match request {
                    Ok(Request::Connect(request)) => {
                        let connection_id = ConnectionId(rng.gen());

                        connections.borrow_mut().insert(connection_id, src, connection_valid_until.borrow().to_owned());

                        let response = Response::Connect(ConnectResponse {
                            connection_id,
                            transaction_id: request.transaction_id,
                        });

                        local_sender.try_send((response, src)).unwrap();
                    }
                    Ok(Request::Announce(request)) => {
                        if connections.borrow().contains(request.connection_id, src) {
                            if access_list.borrow().allows(access_list_mode, &request.info_hash.0) {
                                let request_consumer_index =
                                    (request.info_hash.0[0] as usize) % config.request_workers;

                                if let Err(err) = request_senders.try_send_to(
                                    request_consumer_index,
                                    (response_consumer_index, request, src),
                                ) {
                                    ::log::warn!("request_sender.try_send failed: {:?}", err)
                                }
                            } else {
                                let response = Response::Error(ErrorResponse {
                                    transaction_id: request.transaction_id,
                                    message: "Info hash not allowed".into(),
                                });

                                local_sender.try_send((response, src)).unwrap();
                            }
                        }
                    }
                    Ok(Request::Scrape(request)) => {
                        if connections.borrow().contains(request.connection_id, src) {
                            let response = Response::Error(ErrorResponse {
                                transaction_id: request.transaction_id,
                                message: "Scrape requests not supported".into(),
                            });

                            local_sender.try_send((response, src)).unwrap();
                        }
                    }
                    Err(err) => {
                        ::log::debug!("Request::from_bytes error: {:?}", err);

                        if let RequestParseError::Sendable {
                            connection_id,
                            transaction_id,
                            err,
                        } = err
                        {
                            if connections.borrow().contains(connection_id, src) {
                                let response = ErrorResponse {
                                    transaction_id,
                                    message: err.right_or("Parse error").into(),
                                };

                                local_sender.try_send((response.into(), src)).unwrap();
                            }
                        }
                    }
                }
            }
            Err(err) => {
                ::log::error!("recv_from: {:?}", err);
            }
        }

        yield_if_needed().await;
    }
}

async fn send_responses<S>(socket: Rc<UdpSocket>, mut stream: S)
where
    S: Stream<Item = (Response, SocketAddr)> + ::std::marker::Unpin,
{
    let mut buf = [0u8; MAX_PACKET_SIZE];
    let mut buf = Cursor::new(&mut buf[..]);

    while let Some((response, src)) = stream.next().await {
        buf.set_position(0);

        ::log::debug!("preparing to send response: {:?}", response.clone());

        response
            .write(&mut buf, ip_version_from_ip(src.ip()))
            .expect("write response");

        let position = buf.position() as usize;

        if let Err(err) = socket.send_to(&buf.get_ref()[..position], src).await {
            ::log::info!("send_to failed: {:?}", err);
        }

        yield_if_needed().await;
    }
}

fn ip_version_from_ip(ip: IpAddr) -> IpVersion {
    match ip {
        IpAddr::V4(_) => IpVersion::IPv4,
        IpAddr::V6(ip) => {
            if let [0, 0, 0, 0, 0, 0xffff, ..] = ip.segments() {
                IpVersion::IPv4
            } else {
                IpVersion::IPv6
            }
        }
    }
}
