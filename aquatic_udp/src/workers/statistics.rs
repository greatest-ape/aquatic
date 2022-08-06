use std::fmt::Display;
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use aquatic_common::PanicSentinel;
use crossbeam_channel::Receiver;
use hdrhistogram::Histogram;
use num_format::{Locale, ToFormattedString};
use serde::Serialize;
use time::format_description::well_known::Rfc2822;
use time::OffsetDateTime;
use tinytemplate::TinyTemplate;

use crate::common::*;
use crate::config::Config;

const TEMPLATE_KEY: &str = "statistics";
const TEMPLATE_CONTENTS: &str = include_str!("../../templates/statistics.html");
const STYLESHEET_CONTENTS: &str = concat!(
    "<style>",
    include_str!("../../templates/statistics.css"),
    "</style>"
);

#[derive(Clone, Copy, Debug, Serialize)]
struct PeerHistogramStatistics {
    p0: u64,
    p10: u64,
    p20: u64,
    p30: u64,
    p40: u64,
    p50: u64,
    p60: u64,
    p70: u64,
    p80: u64,
    p90: u64,
    p95: u64,
    p99: u64,
    p100: u64,
}

impl PeerHistogramStatistics {
    fn new(h: &Histogram<u64>) -> Self {
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

impl Display for PeerHistogramStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "p0: {}, p10: {}, p20: {}, p30: {}, p40: {}, p50: {}, p60: {}, p70: {}, p80: {}, p90: {}, p95: {}, p99: {}, p100: {}", self.p0, self.p10, self.p20, self.p30, self.p40, self.p50, self.p60, self.p70, self.p80, self.p90, self.p95, self.p99, self.p100)
    }
}

#[derive(Clone, Copy, Debug)]
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
    peer_histogram: PeerHistogramStatistics,
}

impl CollectedStatistics {
    fn from_shared(
        statistics: &Arc<Statistics>,
        peer_histogram: &Histogram<u64>,
        last: &mut Instant,
    ) -> Self {
        let requests_received = statistics.requests_received.fetch_and(0, Ordering::Relaxed) as f64;
        let responses_sent_connect = statistics
            .responses_sent_connect
            .fetch_and(0, Ordering::Relaxed) as f64;
        let responses_sent_announce = statistics
            .responses_sent_announce
            .fetch_and(0, Ordering::Relaxed) as f64;
        let responses_sent_scrape = statistics
            .responses_sent_scrape
            .fetch_and(0, Ordering::Relaxed) as f64;
        let responses_sent_error = statistics
            .responses_sent_error
            .fetch_and(0, Ordering::Relaxed) as f64;
        let bytes_received = statistics.bytes_received.fetch_and(0, Ordering::Relaxed) as f64;
        let bytes_sent = statistics.bytes_sent.fetch_and(0, Ordering::Relaxed) as f64;
        let num_torrents = Self::sum_atomic_usizes(&statistics.torrents);
        let num_peers = Self::sum_atomic_usizes(&statistics.peers);

        let peer_histogram = PeerHistogramStatistics::new(peer_histogram);

        let now = Instant::now();

        let elapsed = (now - *last).as_secs_f64();

        *last = now;

        Self {
            requests_per_second: requests_received / elapsed,
            responses_per_second_connect: responses_sent_connect / elapsed,
            responses_per_second_announce: responses_sent_announce / elapsed,
            responses_per_second_scrape: responses_sent_scrape / elapsed,
            responses_per_second_error: responses_sent_error / elapsed,
            bytes_received_per_second: bytes_received / elapsed,
            bytes_sent_per_second: bytes_sent / elapsed,
            num_torrents,
            num_peers,
            peer_histogram,
        }
    }

    fn sum_atomic_usizes(values: &[AtomicUsize]) -> usize {
        values.iter().map(|n| n.load(Ordering::Relaxed)).sum()
    }
}

impl Into<FormattedStatistics> for CollectedStatistics {
    fn into(self) -> FormattedStatistics {
        let rx_mbits = self.bytes_received_per_second * 8.0 / 1_000_000.0;
        let tx_mbits = self.bytes_sent_per_second * 8.0 / 1_000_000.0;

        let responses_per_second_total = self.responses_per_second_connect
            + self.responses_per_second_announce
            + self.responses_per_second_scrape
            + self.responses_per_second_error;

        FormattedStatistics {
            requests_per_second: (self.requests_per_second as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_total: (responses_per_second_total as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_connect: (self.responses_per_second_connect as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_announce: (self.responses_per_second_announce as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_scrape: (self.responses_per_second_scrape as usize)
                .to_formatted_string(&Locale::en),
            responses_per_second_error: (self.responses_per_second_error as usize)
                .to_formatted_string(&Locale::en),
            rx_mbits: format!("{:.2}", rx_mbits),
            tx_mbits: format!("{:.2}", tx_mbits),
            num_torrents: self.num_torrents.to_formatted_string(&Locale::en),
            num_peers: self.num_peers.to_formatted_string(&Locale::en),
            peer_histogram: self.peer_histogram,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct FormattedStatistics {
    requests_per_second: String,
    responses_per_second_total: String,
    responses_per_second_connect: String,
    responses_per_second_announce: String,
    responses_per_second_scrape: String,
    responses_per_second_error: String,
    rx_mbits: String,
    tx_mbits: String,
    num_torrents: String,
    num_peers: String,
    peer_histogram: PeerHistogramStatistics,
}

#[derive(Debug, Serialize)]
struct TemplateData {
    stylesheet: String,
    ipv4_active: bool,
    ipv6_active: bool,
    extended_active: bool,
    ipv4: FormattedStatistics,
    ipv6: FormattedStatistics,
    last_updated: String,
    peer_update_interval: String,
}

struct PeerHistograms {
    pending: Vec<Histogram<u64>>,
    last_complete: Histogram<u64>,
}

impl Default for PeerHistograms {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            last_complete: Histogram::new(3).expect("create peer histogram"),
        }
    }
}

impl PeerHistograms {
    fn update(&mut self, config: &Config, histogram: Histogram<u64>) {
        self.pending.push(histogram);

        if self.pending.len() == config.swarm_workers {
            self.last_complete = self.pending.drain(..).sum();
        }
    }

    fn current(&self) -> &Histogram<u64> {
        &self.last_complete
    }
}

pub fn run_statistics_worker(
    _sentinel: PanicSentinel,
    config: Config,
    state: State,
    statistics_receiver: Receiver<StatisticsMessage>,
) {
    let tt = if config.statistics.write_html_to_file {
        let mut tt = TinyTemplate::new();

        if let Err(err) = tt.add_template(TEMPLATE_KEY, TEMPLATE_CONTENTS) {
            ::log::error!("Couldn't parse statistics html template: {:#}", err);

            None
        } else {
            Some(tt)
        }
    } else {
        None
    };

    let mut last_ipv4 = Instant::now();
    let mut last_ipv6 = Instant::now();

    let mut peer_histograms_ipv4 = PeerHistograms::default();
    let mut peer_histograms_ipv6 = PeerHistograms::default();

    loop {
        ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

        for message in statistics_receiver.try_iter() {
            match message {
                StatisticsMessage::Ipv4PeerHistogram(h) => peer_histograms_ipv4.update(&config, h),
                StatisticsMessage::Ipv6PeerHistogram(h) => peer_histograms_ipv6.update(&config, h),
            }
        }

        let statistics_ipv4 = CollectedStatistics::from_shared(
            &state.statistics_ipv4,
            peer_histograms_ipv4.current(),
            &mut last_ipv4,
        )
        .into();
        let statistics_ipv6 = CollectedStatistics::from_shared(
            &state.statistics_ipv6,
            peer_histograms_ipv6.current(),
            &mut last_ipv6,
        )
        .into();

        if config.statistics.print_to_stdout {
            println!("General:");
            println!("  access list entries: {}", state.access_list.load().len());

            if config.network.ipv4_active() {
                println!("IPv4:");
                print_to_stdout(&config, &statistics_ipv4);
            }
            if config.network.ipv6_active() {
                println!("IPv6:");
                print_to_stdout(&config, &statistics_ipv6);
            }

            println!();
        }

        if let Some(tt) = tt.as_ref() {
            let template_data = TemplateData {
                stylesheet: STYLESHEET_CONTENTS.to_string(),
                ipv4_active: config.network.ipv4_active(),
                ipv6_active: config.network.ipv6_active(),
                extended_active: config.statistics.extended,
                ipv4: statistics_ipv4,
                ipv6: statistics_ipv6,
                last_updated: OffsetDateTime::now_utc()
                    .format(&Rfc2822)
                    .unwrap_or("(formatting error)".into()),
                peer_update_interval: format!("{}", config.cleaning.torrent_cleaning_interval),
            };

            if let Err(err) = save_html_to_file(&config, tt, &template_data) {
                ::log::error!("Couldn't save statistics to file: {:#}", err)
            }
        }
    }
}

fn print_to_stdout(config: &Config, statistics: &FormattedStatistics) {
    println!("  requests/second: {:>10}", statistics.requests_per_second);
    println!("  responses/second");
    println!(
        "    total:         {:>10}",
        statistics.responses_per_second_total
    );
    println!(
        "    connect:       {:>10}",
        statistics.responses_per_second_connect
    );
    println!(
        "    announce:      {:>10}",
        statistics.responses_per_second_announce
    );
    println!(
        "    scrape:        {:>10}",
        statistics.responses_per_second_scrape
    );
    println!(
        "    error:         {:>10}",
        statistics.responses_per_second_error
    );
    println!(
        "  bandwidth: {:>7} Mbit/s in, {:7} Mbit/s out",
        statistics.rx_mbits, statistics.tx_mbits,
    );
    println!("  number of torrents: {}", statistics.num_torrents);
    println!(
        "  number of peers: {} (updated every {} seconds)",
        statistics.num_peers, config.cleaning.torrent_cleaning_interval
    );

    if config.statistics.extended {
        println!(
            "  peers per torrent (updated every {} seconds):",
            config.cleaning.torrent_cleaning_interval
        );
        println!("    min            {:>10}", statistics.peer_histogram.p0);
        println!("    p10            {:>10}", statistics.peer_histogram.p10);
        println!("    p20            {:>10}", statistics.peer_histogram.p20);
        println!("    p30            {:>10}", statistics.peer_histogram.p30);
        println!("    p40            {:>10}", statistics.peer_histogram.p40);
        println!("    p50            {:>10}", statistics.peer_histogram.p50);
        println!("    p60            {:>10}", statistics.peer_histogram.p60);
        println!("    p70            {:>10}", statistics.peer_histogram.p70);
        println!("    p80            {:>10}", statistics.peer_histogram.p80);
        println!("    p90            {:>10}", statistics.peer_histogram.p90);
        println!("    p95            {:>10}", statistics.peer_histogram.p95);
        println!("    p99            {:>10}", statistics.peer_histogram.p99);
        println!("    max            {:>10}", statistics.peer_histogram.p100);
    }
}

fn save_html_to_file(
    config: &Config,
    tt: &TinyTemplate,
    template_data: &TemplateData,
) -> anyhow::Result<()> {
    let mut file = File::create(&config.statistics.html_file_path).with_context(|| {
        format!(
            "File path: {}",
            &config.statistics.html_file_path.to_string_lossy()
        )
    })?;

    write!(file, "{}", tt.render(TEMPLATE_KEY, template_data)?)?;

    Ok(())
}
