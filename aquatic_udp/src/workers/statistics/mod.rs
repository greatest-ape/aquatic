mod collector;

use std::fs::File;
use std::io::Write;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::PanicSentinel;
use crossbeam_channel::Receiver;
use serde::Serialize;
use time::format_description::well_known::Rfc2822;
use time::OffsetDateTime;
use tinytemplate::TinyTemplate;

use collector::{CollectedStatistics, StatisticsCollector};

use crate::common::*;
use crate::config::Config;

const TEMPLATE_KEY: &str = "statistics";
const TEMPLATE_CONTENTS: &str = include_str!("../../../templates/statistics.html");
const STYLESHEET_CONTENTS: &str = concat!(
    "<style>",
    include_str!("../../../templates/statistics.css"),
    "</style>"
);

#[derive(Debug, Serialize)]
struct TemplateData {
    stylesheet: String,
    ipv4_active: bool,
    ipv6_active: bool,
    extended_active: bool,
    ipv4: CollectedStatistics,
    ipv6: CollectedStatistics,
    last_updated: String,
    peer_update_interval: String,
}

pub fn run_statistics_worker(
    _sentinel: PanicSentinel,
    config: Config,
    shared_state: State,
    statistics_receiver: Receiver<StatisticsMessage>,
) {
    let opt_tt = if config.statistics.write_html_to_file {
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

    let mut ipv4_collector = StatisticsCollector::new(
        shared_state.statistics_ipv4,
        #[cfg(feature = "prometheus")]
        "4".into(),
    );
    let mut ipv6_collector = StatisticsCollector::new(
        shared_state.statistics_ipv6,
        #[cfg(feature = "prometheus")]
        "6".into(),
    );

    loop {
        ::std::thread::sleep(Duration::from_secs(config.statistics.interval));

        for message in statistics_receiver.try_iter() {
            match message {
                StatisticsMessage::Ipv4PeerHistogram(h) => ipv4_collector.add_histogram(&config, h),
                StatisticsMessage::Ipv6PeerHistogram(h) => ipv6_collector.add_histogram(&config, h),
            }
        }

        let statistics_ipv4 = ipv4_collector.collect_from_shared(
            #[cfg(feature = "prometheus")]
            &config,
        );
        let statistics_ipv6 = ipv6_collector.collect_from_shared(
            #[cfg(feature = "prometheus")]
            &config,
        );

        if config.statistics.print_to_stdout {
            println!("General:");
            println!(
                "  access list entries: {}",
                shared_state.access_list.load().len()
            );

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

        if let Some(tt) = opt_tt.as_ref() {
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

fn print_to_stdout(config: &Config, statistics: &CollectedStatistics) {
    println!(
        "  bandwidth: {:>7} Mbit/s in, {:7} Mbit/s out",
        statistics.rx_mbits, statistics.tx_mbits,
    );
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
    println!("  torrents:        {:>10}", statistics.num_torrents);
    println!(
        "  peers:           {:>10} (updated every {}s)",
        statistics.num_peers, config.cleaning.torrent_cleaning_interval
    );

    if config.statistics.extended {
        println!(
            "  peers per torrent (updated every {}s)",
            config.cleaning.torrent_cleaning_interval
        );
        println!("    min            {:>10}", statistics.peer_histogram.min);
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
        println!("    p99.9          {:>10}", statistics.peer_histogram.p999);
        println!("    max            {:>10}", statistics.peer_histogram.max);
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
