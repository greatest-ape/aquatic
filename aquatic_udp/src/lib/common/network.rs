use std::{net::SocketAddr, time::Instant};

use aquatic_common::access_list::AccessListCache;
use aquatic_common::AHashIndexMap;
use aquatic_common::ValidUntil;
use aquatic_udp_protocol::*;
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

pub struct PendingScrapeResponseMeta {
    num_pending: usize,
    valid_until: ValidUntil,
}

#[derive(Default)]
pub struct PendingScrapeResponseMap(
    AHashIndexMap<TransactionId, (PendingScrapeResponseMeta, PendingScrapeResponse)>,
);

impl PendingScrapeResponseMap {
    pub fn prepare(
        &mut self,
        transaction_id: TransactionId,
        num_pending: usize,
        valid_until: ValidUntil,
    ) {
        let meta = PendingScrapeResponseMeta {
            num_pending,
            valid_until,
        };
        let response = PendingScrapeResponse {
            transaction_id,
            torrent_stats: BTreeMap::new(),
        };

        self.0.insert(transaction_id, (meta, response));
    }

    pub fn add_and_get_finished(&mut self, response: PendingScrapeResponse) -> Option<Response> {
        let finished = if let Some(r) = self.0.get_mut(&response.transaction_id) {
            r.0.num_pending -= 1;

            r.1.torrent_stats.extend(response.torrent_stats.into_iter());

            r.0.num_pending == 0
        } else {
            ::log::warn!("PendingScrapeResponses.add didn't find PendingScrapeResponse in map");

            false
        };

        if finished {
            let response = self.0.remove(&response.transaction_id).unwrap().1;

            Some(Response::Scrape(ScrapeResponse {
                transaction_id: response.transaction_id,
                torrent_stats: response.torrent_stats.into_values().collect(),
            }))
        } else {
            None
        }
    }

    pub fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.0.valid_until.0 > now);
        self.0.shrink_to_fit();
    }
}

pub fn handle_request(
    config: &Config,
    connections: &mut ConnectionMap,
    pending_scrape_responses: &mut PendingScrapeResponseMap,
    access_list_cache: &mut AccessListCache,
    rng: &mut StdRng,
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, SocketAddr)>,
    valid_until: ValidUntil,
    res_request: Result<Request, RequestParseError>,
    src: SocketAddr,
) {
    let access_list_mode = config.access_list.mode;

    match res_request {
        Ok(Request::Connect(request)) => {
            let connection_id = ConnectionId(rng.gen());

            connections.insert(connection_id, src, valid_until);

            let response = Response::Connect(ConnectResponse {
                connection_id,
                transaction_id: request.transaction_id,
            });

            local_responses.push((response, src))
        }
        Ok(Request::Announce(request)) => {
            if connections.contains(request.connection_id, src) {
                if access_list_cache
                    .load()
                    .allows(access_list_mode, &request.info_hash.0)
                {
                    let worker_index =
                        RequestWorkerIndex::from_info_hash(config, request.info_hash);

                    request_sender.try_send_to(
                        worker_index,
                        ConnectedRequest::Announce(request),
                        src,
                    );
                } else {
                    let response = Response::Error(ErrorResponse {
                        transaction_id: request.transaction_id,
                        message: "Info hash not allowed".into(),
                    });

                    local_responses.push((response, src))
                }
            }
        }
        Ok(Request::Scrape(request)) => {
            if connections.contains(request.connection_id, src) {
                let mut requests: AHashIndexMap<RequestWorkerIndex, PendingScrapeRequest> =
                    Default::default();

                let transaction_id = request.transaction_id;

                for (i, info_hash) in request.info_hashes.into_iter().enumerate() {
                    let pending = requests
                        .entry(RequestWorkerIndex::from_info_hash(&config, info_hash))
                        .or_insert_with(|| PendingScrapeRequest {
                            transaction_id,
                            info_hashes: BTreeMap::new(),
                        });

                    pending.info_hashes.insert(i, info_hash);
                }

                pending_scrape_responses.prepare(transaction_id, requests.len(), valid_until);

                for (request_worker_index, request) in requests {
                    request_sender.try_send_to(
                        request_worker_index,
                        ConnectedRequest::Scrape(request),
                        src,
                    );
                }
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

                    local_responses.push((response.into(), src));
                }
            }
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
