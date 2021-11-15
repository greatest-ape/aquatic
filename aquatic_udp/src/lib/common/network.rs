use std::{net::SocketAddr, time::Instant};

use aquatic_common::access_list::AccessListCache;
use aquatic_common::AHashIndexMap;
use aquatic_common::ValidUntil;
use aquatic_udp_protocol::*;
use crossbeam_channel::Sender;
use rand::{prelude::StdRng, Rng};

use crate::common::*;

#[derive(Default)]
pub struct ConnectionMap(AHashIndexMap<(ConnectionId, SocketAddr), ValidUntil>);

impl ConnectionMap {
    pub fn insert(
        &mut self,
        connection_id: ConnectionId,
        socket_addr: SocketAddr,
        valid_until: ValidUntil,
    ) {
        self.0.insert((connection_id, socket_addr), valid_until);
    }

    pub fn contains(&self, connection_id: ConnectionId, socket_addr: SocketAddr) -> bool {
        self.0.contains_key(&(connection_id, socket_addr))
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.0 > now);
        self.0.shrink_to_fit();
    }
}

pub fn handle_request(
    config: &Config,
    connections: &mut ConnectionMap,
    access_list_cache: &mut AccessListCache,
    rng: &mut StdRng,
    request_sender: &Sender<(ConnectedRequest, SocketAddr)>,
    local_responses: &mut Vec<(Response, SocketAddr)>,
    valid_until: ValidUntil,
    res_request: Option<Request>,
    src: SocketAddr,
) {
    let access_list_mode = config.access_list.mode;

    match res_request {
        Some(Request::Connect(request)) => {
            let connection_id = ConnectionId(rng.gen::<i64>().into());

            connections.insert(connection_id, src, valid_until);

            let response = Response::Connect(ConnectResponse {
                action: ConnectAction::new(),
                connection_id,
                transaction_id: request.transaction_id,
            });

            local_responses.push((response, src))
        }
        Some(Request::Announce(request)) => {
            if connections.contains(request.connection_id, src) {
                if access_list_cache
                    .load()
                    .allows(access_list_mode, &request.info_hash.0)
                {
                    if let Err(err) =
                        request_sender.try_send((ConnectedRequest::Announce(request), src))
                    {
                        ::log::warn!("request_sender.try_send failed: {:?}", err)
                    }
                } else {
                    let response = Response::Error(ErrorResponse {
                        fixed: ErrorResponseFixed {
                            action: ErrorAction::new(),
                            transaction_id: request.transaction_id,
                        },
                        message: "Info hash not allowed".into(),
                    });

                    local_responses.push((response, src))
                }
            }
        }
        Some(Request::Scrape(request)) => {
            if connections.contains(request.fixed.connection_id, src) {
                let request = ConnectedRequest::Scrape {
                    request,
                    original_indices: Vec::new(),
                };

                if let Err(err) = request_sender.try_send((request, src)) {
                    ::log::warn!("request_sender.try_send failed: {:?}", err)
                }
            }
        }
        None => {
            ::log::debug!("Request::from_bytes error");
        }
    }
}

pub fn create_socket(config: &Config) -> ::std::net::UdpSocket {
    let socket = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
    }
    .expect("create socket");

    if config.network.only_ipv6 {
        socket.set_only_v6(true).expect("socket: set only ipv6");
    }

    socket.set_reuse_port(true).expect("socket: set reuse port");

    socket
        .set_nonblocking(true)
        .expect("socket: set nonblocking");

    socket
        .bind(&config.network.address.into())
        .unwrap_or_else(|err| panic!("socket: bind to {}: {:?}", config.network.address, err));

    let recv_buffer_size = config.network.socket_recv_buffer_size;

    if recv_buffer_size != 0 {
        if let Err(err) = socket.set_recv_buffer_size(recv_buffer_size) {
            ::log::error!(
                "socket: failed setting recv buffer to {}: {:?}",
                recv_buffer_size,
                err
            );
        }
    }

    socket.into()
}
