/// TODO
/// - forward announce requests to request workers sharded by info hash (with
///   some nice algo to make it difficult for an attacker to know which one
///   they get forwarded to)
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use futures_lite::StreamExt;
use glommio::channels::local_channel::{new_unbounded, LocalReceiver, LocalSender};
use glommio::channels::shared_channel::{SharedReceiver, SharedSender};
use glommio::net::UdpSocket;
use glommio::prelude::*;
use rand::prelude::{Rng, SeedableRng, StdRng};

use aquatic_udp_protocol::{IpVersion, Request, Response};

use crate::common::*;
use crate::config::Config;

pub fn run_socket_worker(
    state: State,
    config: Config,
    request_sender: SharedSender<(AnnounceRequest, SocketAddr)>,
    response_receiver: SharedReceiver<(AnnounceResponse, SocketAddr)>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    LocalExecutorBuilder::default()
        .spawn(|| async move {
            let (local_sender, local_receiver) = new_unbounded();

            let mut socket = UdpSocket::bind(config.network.address).unwrap();

            let recv_buffer_size = config.network.socket_recv_buffer_size;

            if recv_buffer_size != 0 {
                socket.set_buffer_size(recv_buffer_size);
            }

            let socket = Rc::new(socket);

            num_bound_sockets.fetch_add(1, Ordering::SeqCst);

            spawn_local(read_requests(
                config.clone(),
                state.access_list.clone(),
                request_sender,
                local_sender,
                socket.clone(),
            ))
            .await;
            spawn_local(send_responses(response_receiver, local_receiver, socket)).await;
        })
        .expect("failed to spawn local executor")
        .join()
        .unwrap();
}

async fn read_requests(
    config: Config,
    access_list: Arc<AccessList>,
    request_sender: SharedSender<(AnnounceRequest, SocketAddr)>,
    local_sender: LocalSender<(Response, SocketAddr)>,
    socket: Rc<UdpSocket>,
) {
    let request_sender = request_sender.connect().await;

    let mut rng = StdRng::from_entropy();

    let valid_until = ValidUntil::new(config.cleaning.max_connection_age);
    let access_list_mode = config.access_list.mode;

    let mut connections = ConnectionMap::default();

    let mut buf = [0u8; 2048];

    loop {
        match socket.recv_from(&mut buf).await {
            Ok((amt, src)) => {
                let request = Request::from_bytes(&buf[..amt], config.protocol.max_scrape_torrents);

                match request {
                    Ok(Request::Connect(request)) => {
                        let connection_id = ConnectionId(rng.gen());

                        connections.insert(connection_id, src, valid_until);

                        let response = Response::Connect(ConnectResponse {
                            connection_id,
                            transaction_id: request.transaction_id,
                        });

                        local_sender.try_send((response, src));
                    }
                    Ok(Request::Announce(request)) => {
                        if connections.contains(request.connection_id, src) {
                            if access_list.allows(access_list_mode, &request.info_hash.0) {
                                if let Err(err) = request_sender
                                    .try_send((request, src))
                                {
                                    ::log::warn!("request_sender.try_send failed: {:?}", err)
                                }
                            } else {
                                let response = Response::Error(ErrorResponse {
                                    transaction_id: request.transaction_id,
                                    message: "Info hash not allowed".into(),
                                });

                                local_sender.try_send((response, src));
                            }
                        }
                    }
                    Ok(Request::Scrape(request)) => {
                        if connections.contains(request.connection_id, src) {
                            let response = Response::Error(ErrorResponse {
                                transaction_id: request.transaction_id,
                                message: "Scrape requests not supported".into(),
                            });

                            local_sender.try_send((response, src));
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
                            if connections.contains(connection_id, src) {
                                let response = ErrorResponse {
                                    transaction_id,
                                    message: err.right_or("Parse error").into(),
                                };

                                local_sender.try_send((response.into(), src));
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

async fn send_responses(
    response_receiver: SharedReceiver<(AnnounceResponse, SocketAddr)>,
    local_receiver: LocalReceiver<(Response, SocketAddr)>,
    socket: Rc<UdpSocket>,
) {
    let response_receiver = response_receiver.connect().await;

    let mut buf = [0u8; MAX_PACKET_SIZE];
    let mut buf = Cursor::new(&mut buf[..]);

    let mut stream = local_receiver
        .stream()
        .race(response_receiver.map(|(response, addr)| (response.into(), addr)));

    while let Some((response, src)) = stream.next().await {
        buf.set_position(0);

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
