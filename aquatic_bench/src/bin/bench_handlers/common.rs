use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use num_format::{Locale, ToFormattedString};


pub const PARETO_SHAPE: f64 = 0.1;
pub const NUM_INFO_HASHES: usize = 10_000;


pub fn create_progress_bar(name: &str, iterations: u64) -> ProgressBar {
    let t = format!("{:<16} {}", name, "{wide_bar} {pos:>2}/{len:>2}");
    let style = ProgressStyle::default_bar().template(&t);

    ProgressBar::new(iterations).with_style(style)
}


pub fn print_results(
    request_type: &str,
    num_responses: usize,
    duration: Duration,
) {
    let per_second = (
        (num_responses as f64 / (duration.as_micros() as f64 / 1000000.0)
    ) as usize).to_formatted_string(&Locale::se);

    let time_per_request = duration.as_nanos() as f64 / (num_responses as f64);

    println!(
        "{} {:>10} requests/second, {:>8.2} ns/request",
        request_type,
        per_second,
        time_per_request,
    );
}
