use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::marker::PhantomData;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::prelude::AsRawFd;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use aquatic_common::access_list::create_access_list_cache;
use aquatic_common::AHashIndexMap;
use futures_lite::{Stream, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::channels::local_channel::{new_unbounded, LocalSender};
use glommio::enclose;
use glommio::net::UdpSocket;
use glommio::prelude::*;
use glommio::timer::TimerActionRepeat;
use nix::sys::socket::{sendmmsg, ControlMessage, InetAddr, MsgFlags, SendMmsgData};
use nix::sys::uio::IoVec;
use rand::prelude::{Rng, SeedableRng, StdRng};

use aquatic_udp_protocol::{IpVersion, Request, Response};

use super::common::State;

use crate::common::handlers::*;
use crate::common::network::ConnectionMap;
use crate::common::*;
use crate::config::Config;

const PENDING_SCRAPE_MAX_WAIT: u64 = 30;
const MAX_RESPONSES_PER_SYSCALL: usize = 32;

struct PendingScrapeResponse {
    pending_worker_responses: usize,
    valid_until: ValidUntil,
    stats: BTreeMap<usize, TorrentScrapeStatistics>,
}

#[derive(Default)]
struct PendingScrapeResponses(AHashIndexMap<TransactionId, PendingScrapeResponse>);

impl PendingScrapeResponses {
    fn prepare(
        &mut self,
        transaction_id: TransactionId,
        pending_worker_responses: usize,
        valid_until: ValidUntil,
    ) {
        let pending = PendingScrapeResponse {
            pending_worker_responses,
            valid_until,
            stats: BTreeMap::new(),
        };

        self.0.insert(transaction_id, pending);
    }

    fn add_and_get_finished(
        &mut self,
        mut response: ScrapeResponse,
        mut original_indices: Vec<usize>,
    ) -> Option<ScrapeResponse> {
        let finished = if let Some(r) = self.0.get_mut(&response.transaction_id) {
            r.pending_worker_responses -= 1;

            r.stats.extend(
                original_indices
                    .drain(..)
                    .zip(response.torrent_stats.drain(..)),
            );

            r.pending_worker_responses == 0
        } else {
            ::log::warn!("PendingScrapeResponses.add didn't find PendingScrapeResponse in map");

            false
        };

        if finished {
            let PendingScrapeResponse { stats, .. } =
                self.0.remove(&response.transaction_id).unwrap();

            Some(ScrapeResponse {
                transaction_id: response.transaction_id,
                torrent_stats: stats.into_values().collect(),
            })
        } else {
            None
        }
    }

    fn clean(&mut self) {
        let now = Instant::now();

        self.0.retain(|_, v| v.valid_until.0 > now);
        self.0.shrink_to_fit();
    }
}

pub async fn run_socket_worker(
    config: Config,
    state: State,
    request_mesh_builder: MeshBuilder<(usize, ConnectedRequest, SocketAddr), Partial>,
    response_mesh_builder: MeshBuilder<(ConnectedResponse, SocketAddr), Partial>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let (local_sender, local_receiver) = new_unbounded();
    let local_sender = Rc::new(local_sender);

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

    let pending_scrape_responses = Rc::new(RefCell::new(PendingScrapeResponses::default()));

    // Periodically clean pending_scrape_responses
    TimerActionRepeat::repeat(enclose!((pending_scrape_responses) move || {
        enclose!((pending_scrape_responses) move || async move {
            pending_scrape_responses.borrow_mut().clean();

            Some(Duration::from_secs(120))
        })()
    }));

    spawn_local(
        enclose!((local_sender, pending_scrape_responses) read_requests(
            config.clone(),
            state,
            request_senders,
            response_consumer_index,
            local_sender,
            socket.clone(),
            pending_scrape_responses,
        )),
    )
    .detach();

    for (_, receiver) in response_receivers.streams().into_iter() {
        spawn_local(
            enclose!((local_sender, pending_scrape_responses) handle_shared_responses(
                local_sender,
                pending_scrape_responses,
                receiver,
            )),
        )
        .detach();
    }

    let response_sender = Rc::new(RefCell::new(ResponseSender::new(socket)));

    queue_responses_for_sending(response_sender, local_receiver.stream()).await;
}

async fn read_requests(
    config: Config,
    state: State,
    request_senders: Senders<(usize, ConnectedRequest, SocketAddr)>,
    response_consumer_index: usize,
    local_sender: Rc<LocalSender<(Response, SocketAddr)>>,
    socket: Rc<UdpSocket>,
    pending_scrape_responses: Rc<RefCell<PendingScrapeResponses>>,
) {
    let mut rng = StdRng::from_entropy();

    let access_list_mode = config.access_list.mode;

    let max_connection_age = config.cleaning.max_connection_age;
    let connection_valid_until = Rc::new(RefCell::new(ValidUntil::new(max_connection_age)));
    let pending_scrape_valid_until =
        Rc::new(RefCell::new(ValidUntil::new(PENDING_SCRAPE_MAX_WAIT)));
    let connections = Rc::new(RefCell::new(ConnectionMap::default()));
    let mut access_list_cache = create_access_list_cache(&state.access_list);

    // Periodically update connection_valid_until
    TimerActionRepeat::repeat(enclose!((connection_valid_until) move || {
        enclose!((connection_valid_until) move || async move {
            *connection_valid_until.borrow_mut() = ValidUntil::new(max_connection_age);

            Some(Duration::from_secs(1))
        })()
    }));

    // Periodically update pending_scrape_valid_until
    TimerActionRepeat::repeat(enclose!((pending_scrape_valid_until) move || {
        enclose!((pending_scrape_valid_until) move || async move {
            *pending_scrape_valid_until.borrow_mut() = ValidUntil::new(PENDING_SCRAPE_MAX_WAIT);

            Some(Duration::from_secs(10))
        })()
    }));

    // Periodically clean connections
    TimerActionRepeat::repeat(enclose!((config, connections) move || {
        enclose!((config, connections) move || async move {
            connections.borrow_mut().clean();

            Some(Duration::from_secs(config.cleaning.connection_cleaning_interval))
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

                        connections.borrow_mut().insert(
                            connection_id,
                            src,
                            connection_valid_until.borrow().to_owned(),
                        );

                        let response = Response::Connect(ConnectResponse {
                            connection_id,
                            transaction_id: request.transaction_id,
                        });

                        local_sender.try_send((response, src)).unwrap();
                    }
                    Ok(Request::Announce(request)) => {
                        if connections.borrow().contains(request.connection_id, src) {
                            if access_list_cache
                                .load()
                                .allows(access_list_mode, &request.info_hash.0)
                            {
                                let request_consumer_index =
                                    calculate_request_consumer_index(&config, request.info_hash);

                                if let Err(err) = request_senders
                                    .send_to(
                                        request_consumer_index,
                                        (
                                            response_consumer_index,
                                            ConnectedRequest::Announce(request),
                                            src,
                                        ),
                                    )
                                    .await
                                {
                                    ::log::error!("request_sender.try_send failed: {:?}", err)
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
                    Ok(Request::Scrape(ScrapeRequest {
                        transaction_id,
                        connection_id,
                        info_hashes,
                    })) => {
                        if connections.borrow().contains(connection_id, src) {
                            let mut consumer_requests: AHashIndexMap<
                                usize,
                                (ScrapeRequest, Vec<usize>),
                            > = Default::default();

                            for (i, info_hash) in info_hashes.into_iter().enumerate() {
                                let (req, indices) = consumer_requests
                                    .entry(calculate_request_consumer_index(&config, info_hash))
                                    .or_insert_with(|| {
                                        let request = ScrapeRequest {
                                            transaction_id: transaction_id,
                                            connection_id: connection_id,
                                            info_hashes: Vec::new(),
                                        };

                                        (request, Vec::new())
                                    });

                                req.info_hashes.push(info_hash);
                                indices.push(i);
                            }

                            pending_scrape_responses.borrow_mut().prepare(
                                transaction_id,
                                consumer_requests.len(),
                                pending_scrape_valid_until.borrow().to_owned(),
                            );

                            for (consumer_index, (request, original_indices)) in consumer_requests {
                                let request = ConnectedRequest::Scrape {
                                    request,
                                    original_indices,
                                };

                                if let Err(err) = request_senders
                                    .send_to(
                                        consumer_index,
                                        (response_consumer_index, request, src),
                                    )
                                    .await
                                {
                                    ::log::error!("request_sender.send failed: {:?}", err)
                                }
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

async fn handle_shared_responses<S>(
    local_sender: Rc<LocalSender<(Response, SocketAddr)>>,
    pending_scrape_responses: Rc<RefCell<PendingScrapeResponses>>,
    mut stream: S,
) where
    S: Stream<Item = (ConnectedResponse, SocketAddr)> + ::std::marker::Unpin,
{
    while let Some((response, addr)) = stream.next().await {
        let opt_response = match response {
            ConnectedResponse::Announce(response) => Some((Response::Announce(response), addr)),
            ConnectedResponse::Scrape {
                response,
                original_indices,
            } => pending_scrape_responses
                .borrow_mut()
                .add_and_get_finished(response, original_indices)
                .map(|response| (Response::Scrape(response), addr)),
        };

        if let Some((response, addr)) = opt_response {
            local_sender.send((response, addr)).await;
        }

        yield_if_needed().await;
    }
}

async fn queue_responses_for_sending<'a, S>(
    response_sender: Rc<RefCell<ResponseSender>>,
    mut stream: S,
) where
    S: Stream<Item = (Response, SocketAddr)> + ::std::marker::Unpin,
{
    while let Some((response, addr)) = stream.next().await {
        response_sender
            .borrow_mut()
            .queue_and_maybe_send_response(response, addr);

        yield_if_needed().await;
    }
}

struct ResponseSender {
    socket: Rc<UdpSocket>,
    response_buffers: [[u8; MAX_PACKET_SIZE]; MAX_RESPONSES_PER_SYSCALL],
    response_lengths: [usize; MAX_RESPONSES_PER_SYSCALL],
    recepients: [Option<nix::sys::socket::SockAddr>; MAX_RESPONSES_PER_SYSCALL],
    response_index: usize,
}

impl ResponseSender {
    fn new(socket: Rc<UdpSocket>) -> Self {
        Self {
            socket,
            response_buffers: [[0u8; MAX_PACKET_SIZE]; MAX_RESPONSES_PER_SYSCALL],
            response_lengths: [0usize; MAX_RESPONSES_PER_SYSCALL],
            recepients: [None; MAX_RESPONSES_PER_SYSCALL],
            response_index: 0,
        }
    }

    fn queue_and_maybe_send_response(&mut self, response: Response, addr: SocketAddr) {
        let mut buf = Cursor::new(&mut self.response_buffers[self.response_index][..]);

        response
            .write(&mut buf, ip_version_from_ip(addr.ip()))
            .expect("write response");

        self.response_lengths[self.response_index] = buf.position() as usize;
        self.recepients[self.response_index] = Some(Self::convert_socket_addr(&addr));

        if self.response_index == MAX_RESPONSES_PER_SYSCALL - 1 {
            self.force_send();
        } else {
            self.response_index += 1;
        }
    }

    // TODO: call with timer with user-configurable interval
    fn force_send(&mut self) {
        let control_messages: [ControlMessage; 0] = [];
        let num_to_send = self.response_index + 1;

        let mut io_vectors = Vec::with_capacity(num_to_send);

        for i in 0..num_to_send {
            let iov = [IoVec::from_slice(
                &self.response_buffers[i][..self.response_lengths[i]],
            )];

            io_vectors.push(iov);
        }

        let mut messages = Vec::with_capacity(num_to_send);

        for i in 0..num_to_send {
            let message = SendMmsgData {
                iov: &io_vectors[i],
                cmsgs: &control_messages[..],
                addr: self.recepients[i],
                _lt: PhantomData::default(),
            };

            messages.push(message);
        }

        let res = sendmmsg(self.socket.as_raw_fd(), &messages, MsgFlags::MSG_DONTWAIT);

        self.response_index = 0;
    }

    fn convert_socket_addr(addr: &SocketAddr) -> nix::sys::socket::SockAddr {
        nix::sys::socket::SockAddr::Inet(InetAddr::from_std(addr))
    }
}

fn calculate_request_consumer_index(config: &Config, info_hash: InfoHash) -> usize {
    (info_hash.0[0] as usize) % config.request_workers
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
