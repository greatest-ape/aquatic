use std::sync::atomic::Ordering;

use super::common::*;
use crate::config::Config;

pub fn gather_and_print_statistics(state: &State, config: &Config) {
    let interval = config.statistics.interval;

    let requests_received: f64 = state
        .statistics
        .requests_received
        .fetch_and(0, Ordering::SeqCst) as f64;
    let responses_sent: f64 = state
        .statistics
        .responses_sent
        .fetch_and(0, Ordering::SeqCst) as f64;
    let bytes_received: f64 = state
        .statistics
        .bytes_received
        .fetch_and(0, Ordering::SeqCst) as f64;
    let bytes_sent: f64 = state.statistics.bytes_sent.fetch_and(0, Ordering::SeqCst) as f64;

    let requests_per_second = requests_received / interval as f64;
    let responses_per_second: f64 = responses_sent / interval as f64;
    let bytes_received_per_second: f64 = bytes_received / interval as f64;
    let bytes_sent_per_second: f64 = bytes_sent / interval as f64;

    println!(
        "stats: {:.2} requests/second, {:.2} responses/second",
        requests_per_second, responses_per_second
    );

    println!(
        "bandwidth: {:7.2} Mbit/s in, {:7.2} Mbit/s out",
        bytes_received_per_second * 8.0 / 1_000_000.0,
        bytes_sent_per_second * 8.0 / 1_000_000.0,
    );

    println!();
}
