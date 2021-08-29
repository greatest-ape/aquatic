use std::io::{Cursor, ErrorKind};
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use std::vec::Drain;

use crossbeam_channel::{Receiver, Sender};
use async_executor::LocalExecutor;
use futures_lite::future;

use aquatic_udp_protocol::{Request, Response};

use crate::common::*;
use crate::config::Config;
use super::{create_socket, ip_version_from_ip};

pub fn run_socket_worker(
    state: State,
    config: Config,
    token_num: usize,
    request_sender: Sender<(Request, SocketAddr)>,
    response_receiver: Receiver<(Response, SocketAddr)>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let mut buffer = [0u8; MAX_PACKET_SIZE];

    let mut socket = UdpSocket::from_std(create_socket(&config));
    let mut poll = Poll::new().expect("create poll");

    let interests = Interest::READABLE;

    poll.registry()
        .register(&mut socket, Token(token_num), interests)
        .unwrap();

    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let mut events = Events::with_capacity(config.network.poll_event_capacity);

    let mut requests: Vec<(Request, SocketAddr)> = Vec::new();
    let mut local_responses: Vec<(Response, SocketAddr)> = Vec::new();

    let timeout = Duration::from_millis(50);

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter() {
            let token = event.token();

            if (token.0 == token_num) & event.is_readable() {
                read_requests(
                    &state,
                    &config,
                    &mut socket,
                    &mut buffer,
                    &mut requests,
                    &mut local_responses,
                );

                for r in requests.drain(..) {
                    if let Err(err) = request_sender.send(r) {
                        ::log::error!("error sending to request_sender: {}", err);
                    }
                }

                state
                    .statistics
                    .readable_events
                    .fetch_add(1, Ordering::SeqCst);
            }
        }

        send_responses(
            &state,
            &config,
            &mut socket,
            &mut buffer,
            &response_receiver,
            local_responses.drain(..),
        );
    }
}

#[inline]
fn read_requests(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    requests: &mut Vec<(Request, SocketAddr)>,
    local_responses: &mut Vec<(Response, SocketAddr)>,
) {
    let mut requests_received: usize = 0;
    let mut bytes_received: usize = 0;

    loop {
        match socket.recv_from(&mut buffer[..]) {
            Ok((amt, src)) => {
                let request =
                    Request::from_bytes(&buffer[..amt], config.protocol.max_scrape_torrents);

                bytes_received += amt;

                if request.is_ok() {
                    requests_received += 1;
                }

                match request {
                    Ok(request) => {
                        requests.push((request, src));
                    }
                    Err(err) => {
                        ::log::debug!("request_from_bytes error: {:?}", err);

                        if let Some(transaction_id) = err.transaction_id {
                            let opt_message = if err.error.is_some() {
                                Some("Parse error".to_string())
                            } else if let Some(message) = err.message {
                                Some(message)
                            } else {
                                None
                            };

                            if let Some(message) = opt_message {
                                let response = ErrorResponse {
                                    transaction_id,
                                    message,
                                };

                                local_responses.push((response.into(), src));
                            }
                        }
                    }
                }
            }
            Err(err) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break;
                }

                ::log::info!("recv_from error: {}", err);
            }
        }
    }

    if config.statistics.interval != 0 {
        state
            .statistics
            .requests_received
            .fetch_add(requests_received, Ordering::SeqCst);
        state
            .statistics
            .bytes_received
            .fetch_add(bytes_received, Ordering::SeqCst);
    }
}

#[inline]
fn send_responses(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response_receiver: &Receiver<(Response, SocketAddr)>,
    local_responses: Drain<(Response, SocketAddr)>,
) {
    let mut responses_sent: usize = 0;
    let mut bytes_sent: usize = 0;

    let mut cursor = Cursor::new(buffer);

    let response_iterator = local_responses
        .into_iter()
        .chain(response_receiver.try_iter());

    for (response, src) in response_iterator {
        cursor.set_position(0);

        let ip_version = ip_version_from_ip(src.ip());

        response.write(&mut cursor, ip_version).unwrap();

        let amt = cursor.position() as usize;

        match socket.send_to(&cursor.get_ref()[..amt], src) {
            Ok(amt) => {
                responses_sent += 1;
                bytes_sent += amt;
            }
            Err(err) => {
                if err.kind() == ErrorKind::WouldBlock {
                    break;
                }

                ::log::info!("send_to error: {}", err);
            }
        }
    }

    if config.statistics.interval != 0 {
        state
            .statistics
            .responses_sent
            .fetch_add(responses_sent, Ordering::SeqCst);
        state
            .statistics
            .bytes_sent
            .fetch_add(bytes_sent, Ordering::SeqCst);
    }
}