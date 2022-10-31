use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

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
}

impl StatisticsCollector {
    pub fn new(shared: Arc<Statistics>) -> Self {
        Self {
            shared,
            last_update: Instant::now(),
            pending_histograms: Vec::new(),
            last_complete_histogram: Default::default(),
        }
    }

    pub fn add_histogram(&mut self, config: &Config, histogram: Histogram<u64>) {
        self.pending_histograms.push(histogram);

        if self.pending_histograms.len() == config.swarm_workers {
            self.last_complete_histogram =
                PeerHistogramStatistics::new(self.pending_histograms.drain(..).sum());
        }
    }

    pub fn collect_from_shared(&mut self) -> FormattedStatistics {
        let requests_received = Self::fetch_and_reset(&self.shared.requests_received);
        let responses_sent_connect = Self::fetch_and_reset(&self.shared.responses_sent_connect);
        let responses_sent_announce = Self::fetch_and_reset(&self.shared.responses_sent_announce);
        let responses_sent_scrape = Self::fetch_and_reset(&self.shared.responses_sent_scrape);
        let responses_sent_error = Self::fetch_and_reset(&self.shared.responses_sent_error);
        let bytes_received = Self::fetch_and_reset(&self.shared.bytes_received);
        let bytes_sent = Self::fetch_and_reset(&self.shared.bytes_sent);

        let num_torrents = Self::sum_atomic_usizes(&self.shared.torrents);
        let num_peers = Self::sum_atomic_usizes(&self.shared.peers);

        let now = Instant::now();

        let elapsed = (now - self.last_update).as_secs_f64();

        self.last_update = now;

        let collected_statistics = CollectedStatistics {
            requests_per_second: requests_received / elapsed,
            responses_per_second_connect: responses_sent_connect / elapsed,
            responses_per_second_announce: responses_sent_announce / elapsed,
            responses_per_second_scrape: responses_sent_scrape / elapsed,
            responses_per_second_error: responses_sent_error / elapsed,
            bytes_received_per_second: bytes_received / elapsed,
            bytes_sent_per_second: bytes_sent / elapsed,
            num_torrents,
            num_peers,
        };

        FormattedStatistics::new(collected_statistics, self.last_complete_histogram.clone())
    }

    fn sum_atomic_usizes(values: &[AtomicUsize]) -> usize {
        values.iter().map(|n| n.load(Ordering::Relaxed)).sum()
    }

    fn fetch_and_reset(atomic: &AtomicUsize) -> f64 {
        atomic.fetch_and(0, Ordering::Relaxed) as f64
    }
}

#[derive(Clone, Debug)]
struct CollectedStatistics {
    requests_per_second: f64,
    responses_per_second_connect: f64,
    responses_per_second_announce: f64,
    responses_per_second_scrape: f64,
    responses_per_second_error: f64,
    bytes_received_per_second: f64,
    bytes_sent_per_second: f64,
    num_torrents: usize,
    num_peers: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FormattedStatistics {
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

impl FormattedStatistics {
    fn new(statistics: CollectedStatistics, peer_histogram: PeerHistogramStatistics) -> Self {
        let rx_mbits = statistics.bytes_received_per_second * 8.0 / 1_000_000.0;
        let tx_mbits = statistics.bytes_sent_per_second * 8.0 / 1_000_000.0;

        let responses_per_second_total = statistics.responses_per_second_connect
            + statistics.responses_per_second_announce
            + statistics.responses_per_second_scrape
            + statistics.responses_per_second_error;

        FormattedStatistics {
            requests_per_second: (statistics.requests_per_second as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_total: (responses_per_second_total as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_connect: (statistics.responses_per_second_connect as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_announce: (statistics.responses_per_second_announce as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_scrape: (statistics.responses_per_second_scrape as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_error: (statistics.responses_per_second_error as usize)
                .to_formatted_string(&Locale::en),
            rx_mbits: format!("{:.2}", rx_mbits),
            tx_mbits: format!("{:.2}", tx_mbits),
            num_torrents: statistics.num_torrents.to_formatted_string(&Locale::en),
            num_peers: statistics.num_peers.to_formatted_string(&Locale::en),
            peer_histogram,
        }
    }
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct PeerHistogramStatistics {
    pub p0: u64,
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
    pub p100: u64,
}

impl PeerHistogramStatistics {
    fn new(h: Histogram<u64>) -> Self {
        Self {
            p0: h.value_at_percentile(0.0),
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
            p100: h.value_at_percentile(100.0),
        }
    }
}
