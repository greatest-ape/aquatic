use std::sync::atomic::Ordering;
use std::time::Instant;

use hdrhistogram::Histogram;
use num_format::{Locale, ToFormattedString};
use serde::Serialize;

use crate::config::Config;

use super::{IpVersion, Statistics};

#[cfg(feature = "prometheus")]
macro_rules! set_peer_histogram_gauge {
    ($ip_version:expr, $data:expr, $type_label:expr) => {
        ::metrics::gauge!(
            "aquatic_peers_per_torrent",
            "type" => $type_label,
            "ip_version" => $ip_version,
        )
        .set($data as f64);
    };
}

pub struct StatisticsCollector {
    statistics: Statistics,
    ip_version: IpVersion,
    last_update: Instant,
    last_complete_histogram: PeerHistogramStatistics,
}

impl StatisticsCollector {
    pub fn new(statistics: Statistics, ip_version: IpVersion) -> Self {
        Self {
            statistics,
            last_update: Instant::now(),
            last_complete_histogram: Default::default(),
            ip_version,
        }
    }

    pub fn add_histogram(&mut self, histogram: Histogram<u64>) {
        self.last_complete_histogram = PeerHistogramStatistics::new(histogram);
    }

    pub fn collect_from_shared(
        &mut self,
        #[cfg(feature = "prometheus")] config: &Config,
    ) -> CollectedStatistics {
        let mut requests = 0;
        let mut responses_connect: usize = 0;
        let mut responses_announce: usize = 0;
        let mut responses_scrape: usize = 0;
        let mut responses_error: usize = 0;
        let mut bytes_received: usize = 0;
        let mut bytes_sent: usize = 0;

        #[cfg(feature = "prometheus")]
        let ip_version_prometheus_str = self.ip_version.prometheus_str();

        for (i, statistics) in self
            .statistics
            .socket
            .iter()
            .map(|s| s.by_ip_version(self.ip_version))
            .enumerate()
        {
            {
                let n = statistics.requests.fetch_and(0, Ordering::Relaxed);

                requests += n;

                #[cfg(feature = "prometheus")]
                if config.statistics.run_prometheus_endpoint {
                    ::metrics::counter!(
                        "aquatic_requests_total",
                        "ip_version" => ip_version_prometheus_str,
                        "worker_index" => i.to_string(),
                    )
                    .increment(n.try_into().unwrap());
                }
            }
            {
                let n = statistics.responses_connect.fetch_and(0, Ordering::Relaxed);

                responses_connect += n;

                #[cfg(feature = "prometheus")]
                if config.statistics.run_prometheus_endpoint {
                    ::metrics::counter!(
                        "aquatic_responses_total",
                        "type" => "connect",
                        "ip_version" => ip_version_prometheus_str,
                        "worker_index" => i.to_string(),
                    )
                    .increment(n.try_into().unwrap());
                }
            }
            {
                let n = statistics
                    .responses_announce
                    .fetch_and(0, Ordering::Relaxed);

                responses_announce += n;

                #[cfg(feature = "prometheus")]
                if config.statistics.run_prometheus_endpoint {
                    ::metrics::counter!(
                        "aquatic_responses_total",
                        "type" => "announce",
                        "ip_version" => ip_version_prometheus_str,
                        "worker_index" => i.to_string(),
                    )
                    .increment(n.try_into().unwrap());
                }
            }
            {
                let n = statistics.responses_scrape.fetch_and(0, Ordering::Relaxed);

                responses_scrape += n;

                #[cfg(feature = "prometheus")]
                if config.statistics.run_prometheus_endpoint {
                    ::metrics::counter!(
                        "aquatic_responses_total",
                        "type" => "scrape",
                        "ip_version" => ip_version_prometheus_str,
                        "worker_index" => i.to_string(),
                    )
                    .increment(n.try_into().unwrap());
                }
            }
            {
                let n = statistics.responses_error.fetch_and(0, Ordering::Relaxed);

                responses_error += n;

                #[cfg(feature = "prometheus")]
                if config.statistics.run_prometheus_endpoint {
                    ::metrics::counter!(
                        "aquatic_responses_total",
                        "type" => "error",
                        "ip_version" => ip_version_prometheus_str,
                        "worker_index" => i.to_string(),
                    )
                    .increment(n.try_into().unwrap());
                }
            }
            {
                let n = statistics.bytes_received.fetch_and(0, Ordering::Relaxed);

                bytes_received += n;

                #[cfg(feature = "prometheus")]
                if config.statistics.run_prometheus_endpoint {
                    ::metrics::counter!(
                        "aquatic_rx_bytes",
                        "ip_version" => ip_version_prometheus_str,
                        "worker_index" => i.to_string(),
                    )
                    .increment(n.try_into().unwrap());
                }
            }
            {
                let n = statistics.bytes_sent.fetch_and(0, Ordering::Relaxed);

                bytes_sent += n;

                #[cfg(feature = "prometheus")]
                if config.statistics.run_prometheus_endpoint {
                    ::metrics::counter!(
                        "aquatic_tx_bytes",
                        "ip_version" => ip_version_prometheus_str,
                        "worker_index" => i.to_string(),
                    )
                    .increment(n.try_into().unwrap());
                }
            }
        }

        let swarm_statistics = &self.statistics.swarm.by_ip_version(self.ip_version);

        let num_torrents = {
            let num_torrents = swarm_statistics.torrents.load(Ordering::Relaxed);

            #[cfg(feature = "prometheus")]
            if config.statistics.run_prometheus_endpoint {
                ::metrics::gauge!(
                    "aquatic_torrents",
                    "ip_version" => ip_version_prometheus_str,
                )
                .set(num_torrents as f64);
            }

            num_torrents
        };

        let num_peers = {
            let num_peers = swarm_statistics.peers.load(Ordering::Relaxed);

            #[cfg(feature = "prometheus")]
            if config.statistics.run_prometheus_endpoint {
                ::metrics::gauge!(
                    "aquatic_peers",
                    "ip_version" => ip_version_prometheus_str,
                )
                .set(num_peers as f64);
            }

            num_peers
        };

        let elapsed = {
            let now = Instant::now();

            let elapsed = (now - self.last_update).as_secs_f64();

            self.last_update = now;

            elapsed
        };

        #[cfg(feature = "prometheus")]
        if config.statistics.run_prometheus_endpoint && config.statistics.torrent_peer_histograms {
            self.last_complete_histogram
                .update_metrics(ip_version_prometheus_str);
        }

        let requests_per_second = requests as f64 / elapsed;
        let responses_per_second_connect = responses_connect as f64 / elapsed;
        let responses_per_second_announce = responses_announce as f64 / elapsed;
        let responses_per_second_scrape = responses_scrape as f64 / elapsed;
        let responses_per_second_error = responses_error as f64 / elapsed;
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
    fn update_metrics(&self, ip_version: &'static str) {
        set_peer_histogram_gauge!(ip_version, self.min, "min");
        set_peer_histogram_gauge!(ip_version, self.p10, "p10");
        set_peer_histogram_gauge!(ip_version, self.p20, "p20");
        set_peer_histogram_gauge!(ip_version, self.p30, "p30");
        set_peer_histogram_gauge!(ip_version, self.p40, "p40");
        set_peer_histogram_gauge!(ip_version, self.p50, "p50");
        set_peer_histogram_gauge!(ip_version, self.p60, "p60");
        set_peer_histogram_gauge!(ip_version, self.p70, "p70");
        set_peer_histogram_gauge!(ip_version, self.p80, "p80");
        set_peer_histogram_gauge!(ip_version, self.p90, "p90");
        set_peer_histogram_gauge!(ip_version, self.p99, "p99");
        set_peer_histogram_gauge!(ip_version, self.p999, "p999");
        set_peer_histogram_gauge!(ip_version, self.max, "max");
    }
}
