use std::io::Cursor;
use std::sync::atomic::Ordering;
use std::vec::Drain;

use crossbeam_channel::Receiver;
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
) {
    for (response, addr) in local_responses {
        send_response(state, config, socket, buffer, response, addr);
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
            send_response(state, config, socket, buffer, response, addr);
        }
    }
}

fn send_response(
    state: &State,
    config: &Config,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    response: Response,
    addr: CanonicalSocketAddr,
) {
    let mut cursor = Cursor::new(buffer);

    let canonical_addr_is_ipv4 = addr.is_ipv4();

    let addr = if config.network.address.is_ipv4() {
        addr.get_ipv4()
            .expect("found peer ipv6 address while running bound to ipv4 address")
    } else {
        addr.get_ipv6_mapped()
    };

    match response.write(&mut cursor) {
        Ok(()) => {
            let amt = cursor.position() as usize;

            match socket.send_to(&cursor.get_ref()[..amt], addr) {
                Ok(amt) if config.statistics.active() => {
                    let stats = if canonical_addr_is_ipv4 {
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
                Ok(_) => {}
                Err(err) => {
                    ::log::warn!("send_to error: {:#}", err);
                }
            }
        }
        Err(err) => {
            ::log::error!("Response::write error: {:?}", err);
        }
    }
}
