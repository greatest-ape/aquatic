mod requests;
mod responses;
mod storage;

use std::time::{Duration, Instant};

use anyhow::Context;
use crossbeam_channel::Receiver;
use mio::net::UdpSocket;
use mio::{Events, Interest, Poll, Token};
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_common::{
    access_list::create_access_list_cache, privileges::PrivilegeDropper, CanonicalSocketAddr,
    PanicSentinel, ValidUntil,
};
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use requests::read_requests;
use responses::send_responses;
use storage::PendingScrapeResponseSlab;

pub fn run_socket_worker(
    _sentinel: PanicSentinel,
    state: State,
    config: Config,
    token_num: usize,
    mut connection_validator: ConnectionValidator,
    request_sender: ConnectedRequestSender,
    response_receiver: Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    priv_dropper: PrivilegeDropper,
) {
    let mut buffer = [0u8; BUFFER_SIZE];

    let mut socket =
        UdpSocket::from_std(create_socket(&config, priv_dropper).expect("create socket"));
    let mut poll = Poll::new().expect("create poll");

    let interests = Interest::READABLE;

    poll.registry()
        .register(&mut socket, Token(token_num), interests)
        .unwrap();

    let mut events = Events::with_capacity(config.network.poll_event_capacity);
    let mut pending_scrape_responses = PendingScrapeResponseSlab::default();
    let mut access_list_cache = create_access_list_cache(&state.access_list);

    let mut local_responses: Vec<(Response, CanonicalSocketAddr)> = Vec::new();

    let poll_timeout = Duration::from_millis(config.network.poll_timeout_ms);

    let pending_scrape_cleaning_duration =
        Duration::from_secs(config.cleaning.pending_scrape_cleaning_interval);

    let mut pending_scrape_valid_until = ValidUntil::new(config.cleaning.max_pending_scrape_age);
    let mut last_pending_scrape_cleaning = Instant::now();

    let mut iter_counter = 0usize;

    loop {
        poll.poll(&mut events, Some(poll_timeout))
            .expect("failed polling");

        for event in events.iter() {
            let token = event.token();

            if (token.0 == token_num) & event.is_readable() {
                read_requests(
                    &config,
                    &state,
                    &mut connection_validator,
                    &mut pending_scrape_responses,
                    &mut access_list_cache,
                    &mut socket,
                    &mut buffer,
                    &request_sender,
                    &mut local_responses,
                    pending_scrape_valid_until,
                );
            }
        }

        send_responses(
            &state,
            &config,
            &mut socket,
            &mut buffer,
            &response_receiver,
            &mut pending_scrape_responses,
            local_responses.drain(..),
        );

        // Run periodic ValidUntil updates and state cleaning
        if iter_counter % 256 == 0 {
            let now = Instant::now();

            pending_scrape_valid_until =
                ValidUntil::new_with_now(now, config.cleaning.max_pending_scrape_age);

            if now > last_pending_scrape_cleaning + pending_scrape_cleaning_duration {
                pending_scrape_responses.clean();

                last_pending_scrape_cleaning = now;
            }
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn create_socket(
    config: &Config,
    priv_dropper: PrivilegeDropper,
) -> anyhow::Result<::std::net::UdpSocket> {
    let socket = if config.network.address.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?
    };

    if config.network.only_ipv6 {
        socket
            .set_only_v6(true)
            .with_context(|| "socket: set only ipv6")?;
    }

    socket
        .set_reuse_port(true)
        .with_context(|| "socket: set reuse port")?;

    socket
        .set_nonblocking(true)
        .with_context(|| "socket: set nonblocking")?;

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

    socket
        .bind(&config.network.address.into())
        .with_context(|| format!("socket: bind to {}", config.network.address))?;

    priv_dropper.after_socket_creation()?;

    Ok(socket.into())
}
