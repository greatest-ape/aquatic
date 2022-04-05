use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use aquatic_common::PanicSentinel;
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
}

impl CollectedStatistics {
    fn from_shared(statistics: &Arc<Statistics>, last: &mut Instant) -> Self {
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
}

#[derive(Debug, Serialize)]
struct TemplateData {
    stylesheet: String,
    ipv4_active: bool,
    ipv6_active: bool,
    ipv4: FormattedStatistics,
    ipv6: FormattedStatistics,
    last_updated: String,
    peer_update_interval: String,
}

pub fn run_statistics_worker(_sentinel: PanicSentinel, config: Config, state: State) {
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

    loop {
        ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

        let statistics_ipv4 =
            CollectedStatistics::from_shared(&state.statistics_ipv4, &mut last_ipv4).into();
        let statistics_ipv6 =
            CollectedStatistics::from_shared(&state.statistics_ipv6, &mut last_ipv6).into();

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
