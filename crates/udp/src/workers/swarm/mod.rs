mod storage;

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use aquatic_common::ServerStartInstant;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_common::{CanonicalSocketAddr, ValidUntil};

use crate::common::*;
use crate::config::Config;

use storage::TorrentMaps;

pub struct SwarmWorker {
    pub config: Config,
    pub state: State,
    pub server_start_instant: ServerStartInstant,
    pub request_receiver: Receiver<(SocketWorkerIndex, ConnectedRequest, CanonicalSocketAddr)>,
    pub response_sender: ConnectedResponseSender,
    pub statistics_sender: Sender<StatisticsMessage>,
    pub worker_index: SwarmWorkerIndex,
}

impl SwarmWorker {
    pub fn run(&mut self) -> anyhow::Result<()> {
        let mut torrents = TorrentMaps::default();
        let mut rng = SmallRng::from_entropy();

        let timeout = Duration::from_millis(self.config.request_channel_recv_timeout_ms);
        let mut peer_valid_until =
            ValidUntil::new(self.server_start_instant, self.config.cleaning.max_peer_age);

        let cleaning_interval = Duration::from_secs(self.config.cleaning.torrent_cleaning_interval);
        let statistics_update_interval = Duration::from_secs(self.config.statistics.interval);

        let mut last_cleaning = Instant::now();
        let mut last_statistics_update = Instant::now();

        let mut iter_counter = 0usize;

        loop {
            if let Ok((sender_index, request, src)) = self.request_receiver.recv_timeout(timeout) {
                // It is OK to block here as long as we don't also do blocking
                // sends in socket workers (doing both could cause a deadlock)
                match (request, src.get().ip()) {
                    (ConnectedRequest::Announce(request), IpAddr::V4(ip)) => {
                        let response = torrents
                            .ipv4
                            .0
                            .entry(request.info_hash)
                            .or_default()
                            .announce(
                                &self.config,
                                &self.statistics_sender,
                                &mut rng,
                                &request,
                                ip.into(),
                                peer_valid_until,
                            );

                        // It doesn't matter which socket worker receives announce responses
                        self.response_sender
                            .send_to_any(src, ConnectedResponse::AnnounceIpv4(response))
                            .expect("swarm response channel is closed");
                    }
                    (ConnectedRequest::Announce(request), IpAddr::V6(ip)) => {
                        let response = torrents
                            .ipv6
                            .0
                            .entry(request.info_hash)
                            .or_default()
                            .announce(
                                &self.config,
                                &self.statistics_sender,
                                &mut rng,
                                &request,
                                ip.into(),
                                peer_valid_until,
                            );

                        // It doesn't matter which socket worker receives announce responses
                        self.response_sender
                            .send_to_any(src, ConnectedResponse::AnnounceIpv6(response))
                            .expect("swarm response channel is closed");
                    }
                    (ConnectedRequest::Scrape(request), IpAddr::V4(_)) => {
                        let response = torrents.ipv4.scrape(request);

                        self.response_sender
                            .send_to(sender_index, src, ConnectedResponse::Scrape(response))
                            .expect("swarm response channel is closed");
                    }
                    (ConnectedRequest::Scrape(request), IpAddr::V6(_)) => {
                        let response = torrents.ipv6.scrape(request);

                        self.response_sender
                            .send_to(sender_index, src, ConnectedResponse::Scrape(response))
                            .expect("swarm response channel is closed");
                    }
                };
            }

            // Run periodic tasks
            if iter_counter % 128 == 0 {
                let now = Instant::now();

                peer_valid_until =
                    ValidUntil::new(self.server_start_instant, self.config.cleaning.max_peer_age);

                if now > last_cleaning + cleaning_interval {
                    torrents.clean_and_update_statistics(
                        &self.config,
                        &self.state,
                        &self.statistics_sender,
                        &self.state.access_list,
                        self.server_start_instant,
                        self.worker_index,
                    );

                    last_cleaning = now;
                }
                if self.config.statistics.active()
                    && now > last_statistics_update + statistics_update_interval
                {
                    self.state.statistics_ipv4.torrents[self.worker_index.0]
                        .store(torrents.ipv4.num_torrents(), Ordering::Release);
                    self.state.statistics_ipv6.torrents[self.worker_index.0]
                        .store(torrents.ipv6.num_torrents(), Ordering::Release);

                    last_statistics_update = now;
                }
            }

            iter_counter = iter_counter.wrapping_add(1);
        }
    }
}
