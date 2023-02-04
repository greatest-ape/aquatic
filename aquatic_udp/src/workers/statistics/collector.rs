use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use num_format::{Locale, ToFormattedString};
use serde::Serialize;

use crate::common::Statistics;

pub struct StatisticsCollector {
    shared: Arc<Statistics>,
    last_update: Instant,
}

impl StatisticsCollector {
    pub fn new(shared: Arc<Statistics>) -> Self {
        Self {
            shared,
            last_update: Instant::now(),
        }
    }
    pub fn collect_from_shared(&mut self) -> CollectedStatistics {
        let requests_received = Self::fetch_and_reset(&self.shared.requests_received);
        let responses_sent_connect = Self::fetch_and_reset(&self.shared.responses_sent_connect);
        let responses_sent_announce = Self::fetch_and_reset(&self.shared.responses_sent_announce);
        let responses_sent_scrape = Self::fetch_and_reset(&self.shared.responses_sent_scrape);
        let responses_sent_error = Self::fetch_and_reset(&self.shared.responses_sent_error);
        let bytes_received = Self::fetch_and_reset(&self.shared.bytes_received);
        let bytes_sent = Self::fetch_and_reset(&self.shared.bytes_sent);
        let num_torrents = Self::sum_atomic_usizes(&self.shared.torrents);
        let num_peers = Self::sum_atomic_usizes(&self.shared.peers);

        let elapsed = {
            let now = Instant::now();

            let elapsed = (now - self.last_update).as_secs_f64();

            self.last_update = now;

            elapsed
        };

        let requests_per_second = requests_received / elapsed;
        let responses_per_second_connect = responses_sent_connect / elapsed;
        let responses_per_second_announce = responses_sent_announce / elapsed;
        let responses_per_second_scrape = responses_sent_scrape / elapsed;
        let responses_per_second_error = responses_sent_error / elapsed;
        let bytes_received_per_second = bytes_received / elapsed;
        let bytes_sent_per_second = bytes_sent / elapsed;

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
        }
    }

    fn sum_atomic_usizes(values: &[AtomicUsize]) -> usize {
        values.iter().map(|n| n.load(Ordering::Relaxed)).sum()
    }

    fn fetch_and_reset(atomic: &AtomicUsize) -> f64 {
        atomic.fetch_and(0, Ordering::Relaxed) as f64
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
}
