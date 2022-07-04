use std::io::{Cursor, ErrorKind};
use std::sync::atomic::Ordering;
use std::vec::Drain;

use crossbeam_channel::Receiver;
use libc::ENOBUFS;
use mio::net::UdpSocket;

use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use super::storage::PendingScrapeResponseSlab;

pub fn send_responses(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response_receiver: &Receiver<(ConnectedResponse, CanonicalSocketAddr)>,
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    local_responses: Drain<(Response, CanonicalSocketAddr)>,
    opt_resend_buffer: &mut Option<Vec<(Response, CanonicalSocketAddr)>>,
) {
    if let Some(resend_buffer) = opt_resend_buffer {
        for (response, addr) in resend_buffer.drain(..) {
            send_response(state, config, socket, buffer, response, addr, &mut None);
        }
    }

    for (response, addr) in local_responses {
        send_response(
            state,
            config,
            socket,
            buffer,
            response,
            addr,
            opt_resend_buffer,
        );
    }

    for (response, addr) in response_receiver.try_iter() {
        let opt_response = match response {
            ConnectedResponse::Scrape(r) => pending_scrape_responses
                .add_and_get_finished(r)
                .map(Response::Scrape),
            ConnectedResponse::AnnounceIpv4(r) => Some(Response::AnnounceIpv4(r)),
            ConnectedResponse::AnnounceIpv6(r) => Some(Response::AnnounceIpv6(r)),
        };

        if let Some(response) = opt_response {
            send_response(
                state,
                config,
                socket,
                buffer,
                response,
                addr,
                opt_resend_buffer,
            );
        }
    }
}

fn send_response(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response: Response,
    canonical_addr: CanonicalSocketAddr,
    resend_buffer: &mut Option<Vec<(Response, CanonicalSocketAddr)>>,
) {
    let mut cursor = Cursor::new(buffer);

    if let Err(err) = response.write(&mut cursor) {
        ::log::error!("Converting response to bytes failed: {:#}", err);

        return;
    }

    let bytes_written = cursor.position() as usize;

    let addr = if config.network.address.is_ipv4() {
        canonical_addr
            .get_ipv4()
            .expect("found peer ipv6 address while running bound to ipv4 address")
    } else {
        canonical_addr.get_ipv6_mapped()
    };

    match socket.send_to(&cursor.get_ref()[..bytes_written], addr) {
        Ok(amt) if config.statistics.active() => {
            let stats = if canonical_addr.is_ipv4() {
                &state.statistics_ipv4
            } else {
                &state.statistics_ipv6
            };

            stats.bytes_sent.fetch_add(amt, Ordering::Relaxed);

            match response {
                Response::Connect(_) => {
                    stats.responses_sent_connect.fetch_add(1, Ordering::Relaxed);
                }
                Response::AnnounceIpv4(_) | Response::AnnounceIpv6(_) => {
                    stats
                        .responses_sent_announce
                        .fetch_add(1, Ordering::Relaxed);
                }
                Response::Scrape(_) => {
                    stats.responses_sent_scrape.fetch_add(1, Ordering::Relaxed);
                }
                Response::Error(_) => {
                    stats.responses_sent_error.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Ok(_) => (),
        Err(err) => {
            match resend_buffer {
                Some(resend_buffer)
                    if (err.raw_os_error() == Some(ENOBUFS))
                        || (err.kind() == ErrorKind::WouldBlock) =>
                {
                    if resend_buffer.len() < config.network.resend_buffer_max_len {
                        ::log::info!("Adding response to resend queue, since sending it to {} failed with: {:#}", addr, err);

                        resend_buffer.push((response, canonical_addr));
                    } else {
                        ::log::warn!("Response resend buffer full, dropping response");
                    }
                }
                _ => {
                    ::log::warn!("Sending response to {} failed: {:#}", addr, err);
                }
            }
        }
    }
}
