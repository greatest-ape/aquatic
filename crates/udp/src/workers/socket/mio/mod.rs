mod socket;

use std::time::Duration;

use anyhow::Context;
use aquatic_common::access_list::AccessListCache;
use crossbeam_channel::Sender;
use mio::{Events, Interest, Poll, Token};

use aquatic_common::{
    access_list::create_access_list_cache, privileges::PrivilegeDropper, CanonicalSocketAddr,
    ValidUntil,
};
use aquatic_udp_protocol::*;
use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::common::*;
use crate::config::Config;

use socket::Socket;

use super::validator::ConnectionValidator;
use super::{EXTRA_PACKET_SIZE_IPV4, EXTRA_PACKET_SIZE_IPV6};

const TOKEN_V4: Token = Token(0);
const TOKEN_V6: Token = Token(1);

pub fn run(
    config: Config,
    shared_state: State,
    statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
    statistics_sender: Sender<StatisticsMessage>,
    validator: ConnectionValidator,
    mut priv_droppers: Vec<PrivilegeDropper>,
) -> anyhow::Result<()> {
    let mut opt_socket_ipv4 = if config.network.use_ipv4 {
        let priv_dropper = priv_droppers.pop().expect("not enough privilege droppers");

        Some(Socket::<self::socket::Ipv4>::create(&config, priv_dropper)?)
    } else {
        None
    };
    let mut opt_socket_ipv6 = if config.network.use_ipv6 {
        let priv_dropper = priv_droppers.pop().expect("not enough privilege droppers");

        Some(Socket::<self::socket::Ipv6>::create(&config, priv_dropper)?)
    } else {
        None
    };

    let access_list_cache = create_access_list_cache(&shared_state.access_list);
    let peer_valid_until = ValidUntil::new(
        shared_state.server_start_instant,
        config.cleaning.max_peer_age,
    );

    let mut shared = WorkerSharedData {
        config,
        shared_state,
        statistics,
        statistics_sender,
        validator,
        access_list_cache,
        buffer: [0; BUFFER_SIZE],
        rng: SmallRng::from_entropy(),
        peer_valid_until,
    };

    let mut events = Events::with_capacity(2);
    let mut poll = Poll::new().context("create poll")?;

    if let Some(socket) = opt_socket_ipv4.as_mut() {
        poll.registry()
            .register(&mut socket.socket, TOKEN_V4, Interest::READABLE)
            .context("register poll")?;
    }
    if let Some(socket) = opt_socket_ipv6.as_mut() {
        poll.registry()
            .register(&mut socket.socket, TOKEN_V6, Interest::READABLE)
            .context("register poll")?;
    }

    let poll_timeout = Duration::from_millis(shared.config.network.poll_timeout_ms);

    let mut iter_counter = 0u64;

    loop {
        poll.poll(&mut events, Some(poll_timeout)).context("poll")?;

        for event in events.iter() {
            if event.is_readable() {
                match event.token() {
                    TOKEN_V4 => {
                        if let Some(socket) = opt_socket_ipv4.as_mut() {
                            socket.read_and_handle_requests(&mut shared);
                        }
                    }
                    TOKEN_V6 => {
                        if let Some(socket) = opt_socket_ipv6.as_mut() {
                            socket.read_and_handle_requests(&mut shared);
                        }
                    }
                    _ => (),
                }
            }
        }

        if let Some(socket) = opt_socket_ipv4.as_mut() {
            socket.resend_failed(&mut shared);
        }
        if let Some(socket) = opt_socket_ipv6.as_mut() {
            socket.resend_failed(&mut shared);
        }

        if iter_counter % 256 == 0 {
            shared.validator.update_elapsed();

            shared.peer_valid_until = ValidUntil::new(
                shared.shared_state.server_start_instant,
                shared.config.cleaning.max_peer_age,
            );
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

pub struct WorkerSharedData {
    config: Config,
    shared_state: State,
    statistics: CachePaddedArc<IpVersionStatistics<SocketWorkerStatistics>>,
    statistics_sender: Sender<StatisticsMessage>,
    access_list_cache: AccessListCache,
    validator: ConnectionValidator,
    buffer: [u8; BUFFER_SIZE],
    rng: SmallRng,
    peer_valid_until: ValidUntil,
}

impl WorkerSharedData {
    fn handle_request(&mut self, request: Request, src: CanonicalSocketAddr) -> Option<Response> {
        let access_list_mode = self.config.access_list.mode;

        match request {
            Request::Connect(request) => {
                return Some(Response::Connect(ConnectResponse {
                    connection_id: self.validator.create_connection_id(src),
                    transaction_id: request.transaction_id,
                }));
            }
            Request::Announce(request) => {
                if self
                    .validator
                    .connection_id_valid(src, request.connection_id)
                {
                    if self
                        .access_list_cache
                        .load()
                        .allows(access_list_mode, &request.info_hash.0)
                    {
                        let response = self.shared_state.torrent_maps.announce(
                            &self.config,
                            &self.statistics_sender,
                            &mut self.rng,
                            &request,
                            src,
                            self.peer_valid_until,
                        );

                        return Some(response);
                    } else {
                        return Some(Response::Error(ErrorResponse {
                            transaction_id: request.transaction_id,
                            message: "Info hash not allowed".into(),
                        }));
                    }
                }
            }
            Request::Scrape(request) => {
                if self
                    .validator
                    .connection_id_valid(src, request.connection_id)
                {
                    return Some(Response::Scrape(
                        self.shared_state.torrent_maps.scrape(request, src),
                    ));
                }
            }
        }

        None
    }
}
