use std::io::ErrorKind;
use std::sync::atomic::Ordering;

use mio::net::UdpSocket;

use aquatic_common::access_list::AccessListCache;
use aquatic_common::CanonicalSocketAddr;
use aquatic_common::ValidUntil;
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

use super::storage::PendingScrapeResponseSlab;

pub fn read_requests(
    config: &Config,
    state: &State,
    connection_validator: &mut ConnectionValidator,
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    access_list_cache: &mut AccessListCache,
    socket: &mut UdpSocket,
    buffer: &mut [u8],
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, CanonicalSocketAddr)>,
    pending_scrape_valid_until: ValidUntil,
) {
    let mut requests_received_ipv4: usize = 0;
    let mut requests_received_ipv6: usize = 0;
    let mut bytes_received_ipv4: usize = 0;
    let mut bytes_received_ipv6 = 0;

    loop {
        match socket.recv_from(&mut buffer[..]) {
            Ok((amt, src)) => {
                let res_request =
                    Request::from_bytes(&buffer[..amt], config.protocol.max_scrape_torrents);

                let src = CanonicalSocketAddr::new(src);

                // Update statistics for converted address
                if src.is_ipv4() {
                    if res_request.is_ok() {
                        requests_received_ipv4 += 1;
                    }
                    bytes_received_ipv4 += amt;
                } else {
                    if res_request.is_ok() {
                        requests_received_ipv6 += 1;
                    }
                    bytes_received_ipv6 += amt;
                }

                handle_request(
                    config,
                    connection_validator,
                    pending_scrape_responses,
                    access_list_cache,
                    request_sender,
                    local_responses,
                    pending_scrape_valid_until,
                    res_request,
                    src,
                );
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                break;
            }
            Err(err) => {
                ::log::warn!("recv_from error: {:#}", err);
            }
        }
    }

    if config.statistics.active() {
        state
            .statistics_ipv4
            .requests_received
            .fetch_add(requests_received_ipv4, Ordering::Release);
        state
            .statistics_ipv6
            .requests_received
            .fetch_add(requests_received_ipv6, Ordering::Release);
        state
            .statistics_ipv4
            .bytes_received
            .fetch_add(bytes_received_ipv4, Ordering::Release);
        state
            .statistics_ipv6
            .bytes_received
            .fetch_add(bytes_received_ipv6, Ordering::Release);
    }
}

fn handle_request(
    config: &Config,
    connection_validator: &mut ConnectionValidator,
    pending_scrape_responses: &mut PendingScrapeResponseSlab,
    access_list_cache: &mut AccessListCache,
    request_sender: &ConnectedRequestSender,
    local_responses: &mut Vec<(Response, CanonicalSocketAddr)>,
    pending_scrape_valid_until: ValidUntil,
    res_request: Result<Request, RequestParseError>,
    src: CanonicalSocketAddr,
) {
    let access_list_mode = config.access_list.mode;

    match res_request {
        Ok(Request::Connect(request)) => {
            let connection_id = connection_validator.create_connection_id(src);

            let response = Response::Connect(ConnectResponse {
                connection_id,
                transaction_id: request.transaction_id,
            });

            local_responses.push((response, src))
        }
        Ok(Request::Announce(request)) => {
            if connection_validator.connection_id_valid(src, request.connection_id) {
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
            if connection_validator.connection_id_valid(src, request.connection_id) {
                let split_requests = pending_scrape_responses.prepare_split_requests(
                    config,
                    request,
                    pending_scrape_valid_until,
                );

                for (request_worker_index, request) in split_requests {
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
                if connection_validator.connection_id_valid(src, connection_id) {
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
