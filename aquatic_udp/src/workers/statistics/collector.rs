use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::Receiver;
use hdrhistogram::Histogram;
use num_format::{Locale, ToFormattedString};
use serde::Serialize;

use crate::common::Statistics;
use crate::config::Config;

pub struct StatisticsCollector {
    shared: Arc<Statistics>,
    last_update: Instant,
    pending_histograms: Vec<Histogram<u64>>,
    last_complete_histogram: PeerHistogramStatistics,
    #[cfg(feature = "prometheus")]
    ip_version: String,
}

impl StatisticsCollector {
    pub fn new(shared: Arc<Statistics>, #[cfg(feature = "prometheus")] ip_version: String) -> Self {
        Self {
            shared,
            last_update: Instant::now(),
            pending_histograms: Vec::new(),
            last_complete_histogram: Default::default(),
            #[cfg(feature = "prometheus")]
            ip_version,
        }
    }

    pub fn add_histogram(&mut self, config: &Config, histogram: Histogram<u64>) {
        self.pending_histograms.push(histogram);

        if self.pending_histograms.len() == config.swarm_workers {
            self.last_complete_histogram =
                PeerHistogramStatistics::new(self.pending_histograms.drain(..).sum());
        }
    }

    pub fn collect_from_shared(
        &mut self,
        #[cfg(feature = "prometheus")] config: &Config,
    ) -> CollectedStatistics {
        let requests_received = Self::fetch_and_reset(&self.shared.requests_received);
        let responses_sent_connect = Self::fetch_and_reset(&self.shared.responses_sent_connect);
        let responses_sent_announce = Self::fetch_and_reset(&self.shared.responses_sent_announce);
        let responses_sent_scrape = Self::fetch_and_reset(&self.shared.responses_sent_scrape);
        let responses_sent_error = Self::fetch_and_reset(&self.shared.responses_sent_error);

        let bytes_received = Self::fetch_and_reset(&self.shared.bytes_received);
        let bytes_sent = Self::fetch_and_reset(&self.shared.bytes_sent);

        let num_torrents_by_worker: Vec<usize> = self
            .shared
            .torrents
            .iter()
            .map(|n| n.load(Ordering::Relaxed))
            .collect();
        let num_peers_by_worker: Vec<usize> = self
            .shared
            .peers
            .iter()
            .map(|n| n.load(Ordering::Relaxed))
            .collect();

        let elapsed = {
            let now = Instant::now();

            let elapsed = (now - self.last_update).as_secs_f64();

            self.last_update = now;

            elapsed
        };

        #[cfg(feature = "prometheus")]
        if config.statistics.run_prometheus_endpoint {
            ::metrics::counter!(
                "aquatic_requests_total",
                requests_received.try_into().unwrap(),
                "ip_version" => self.ip_version.clone(),
            );
            ::metrics::counter!(
                "aquatic_responses_total",
                responses_sent_connect.try_into().unwrap(),
                "type" => "connect",
                "ip_version" => self.ip_version.clone(),
            );
            ::metrics::counter!(
                "aquatic_responses_total",
                responses_sent_announce.try_into().unwrap(),
                "type" => "announce",
                "ip_version" => self.ip_version.clone(),
            );
            ::metrics::counter!(
                "aquatic_responses_total",
                responses_sent_scrape.try_into().unwrap(),
                "type" => "scrape",
                "ip_version" => self.ip_version.clone(),
            );
            ::metrics::counter!(
                "aquatic_responses_total",
                responses_sent_error.try_into().unwrap(),
                "type" => "error",
                "ip_version" => self.ip_version.clone(),
            );
            ::metrics::counter!(
                "aquatic_rx_bytes",
                bytes_received.try_into().unwrap(),
                "ip_version" => self.ip_version.clone(),
            );
            ::metrics::counter!(
                "aquatic_tx_bytes",
                bytes_sent.try_into().unwrap(),
                "ip_version" => self.ip_version.clone(),
            );

            for (worker_index, n) in num_torrents_by_worker.iter().copied().enumerate() {
                ::metrics::gauge!(
                    "aquatic_torrents",
                    n as f64,
                    "ip_version" => self.ip_version.clone(),
                    "worker_index" => worker_index.to_string(),
                );
            }
            for (worker_index, n) in num_peers_by_worker.iter().copied().enumerate() {
                ::metrics::gauge!(
                    "aquatic_peers",
                    n as f64,
                    "ip_version" => self.ip_version.clone(),
                    "worker_index" => worker_index.to_string(),
                );
            }

            if config.statistics.extended {
                self.last_complete_histogram
                    .update_metrics(self.ip_version.clone());
            }
        }

        let num_peers: usize = num_peers_by_worker.into_iter().sum();
        let num_torrents: usize = num_torrents_by_worker.into_iter().sum();

        let requests_per_second = requests_received as f64 / elapsed;
        let responses_per_second_connect = responses_sent_connect as f64 / elapsed;
        let responses_per_second_announce = responses_sent_announce as f64 / elapsed;
        let responses_per_second_scrape = responses_sent_scrape as f64 / elapsed;
        let responses_per_second_error = responses_sent_error as f64 / elapsed;
        let bytes_received_per_second = bytes_received as f64 / elapsed;
        let bytes_sent_per_second = bytes_sent as f64 / elapsed;

        let responses_per_second_total = responses_per_second_connect
            + responses_per_second_announce
            + responses_per_second_scrape
            + responses_per_second_error;

        CollectedStatistics {
            requests_per_second: (requests_per_second as usize).to_formatted_string(&Locale::en),
            responses_per_second_total: (responses_per_second_total as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_connect: (responses_per_second_connect as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_announce: (responses_per_second_announce as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_scrape: (responses_per_second_scrape as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_error: (responses_per_second_error as usize)
                .to_formatted_string(&Locale::en),
            rx_mbits: format!("{:.2}", bytes_received_per_second * 8.0 / 1_000_000.0),
            tx_mbits: format!("{:.2}", bytes_sent_per_second * 8.0 / 1_000_000.0),
            num_torrents: num_torrents.to_formatted_string(&Locale::en),
            num_peers: num_peers.to_formatted_string(&Locale::en),
            peer_histogram: self.last_complete_histogram.clone(),
        }
    }

    fn fetch_and_reset(atomic: &AtomicUsize) -> usize {
        atomic.fetch_and(0, Ordering::Relaxed)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CollectedStatistics {
    pub requests_per_second: String,
    pub responses_per_second_total: String,
    pub responses_per_second_connect: String,
    pub responses_per_second_announce: String,
    pub responses_per_second_scrape: String,
    pub responses_per_second_error: String,
    pub rx_mbits: String,
    pub tx_mbits: String,
    pub num_torrents: String,
    pub num_peers: String,
    pub peer_histogram: PeerHistogramStatistics,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct PeerHistogramStatistics {
    pub min: u64,
    pub p10: u64,
    pub p20: u64,
    pub p30: u64,
    pub p40: u64,
    pub p50: u64,
    pub p60: u64,
    pub p70: u64,
    pub p80: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

impl PeerHistogramStatistics {
    fn new(h: Histogram<u64>) -> Self {
        Self {
            min: h.min(),
            p10: h.value_at_percentile(10.0),
            p20: h.value_at_percentile(20.0),
            p30: h.value_at_percentile(30.0),
            p40: h.value_at_percentile(40.0),
            p50: h.value_at_percentile(50.0),
            p60: h.value_at_percentile(60.0),
            p70: h.value_at_percentile(70.0),
            p80: h.value_at_percentile(80.0),
            p90: h.value_at_percentile(90.0),
            p95: h.value_at_percentile(95.0),
            p99: h.value_at_percentile(99.0),
            p999: h.value_at_percentile(99.9),
            max: h.max(),
        }
    }

    #[cfg(feature = "prometheus")]
    fn update_metrics(&self, ip_version: String) {
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.min as f64,
            "type" => "max",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p10 as f64,
            "type" => "p10",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p20 as f64,
            "type" => "p20",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p30 as f64,
            "type" => "p30",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p40 as f64,
            "type" => "p40",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p50 as f64,
            "type" => "p50",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p60 as f64,
            "type" => "p60",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p70 as f64,
            "type" => "p70",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p80 as f64,
            "type" => "p80",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p90 as f64,
            "type" => "p90",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p99 as f64,
            "type" => "p99",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.p999 as f64,
            "type" => "p99.9",
            "ip_version" => ip_version.clone(),
        );
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            self.max as f64,
            "type" => "max",
            "ip_version" => ip_version.clone(),
        );
    }
}
