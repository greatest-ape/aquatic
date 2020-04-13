
use indicatif::{ProgressBar, ProgressStyle};


pub const PARETO_SHAPE: f64 = 0.1;
pub const NUM_INFO_HASHES: usize = 10_000;


pub fn create_progress_bar(name: &str, iterations: u64) -> ProgressBar {
    let t = format!("{:<16} {}", name, "{wide_bar} {pos:>2}/{len:>2}");
    let style = ProgressStyle::default_bar().template(&t);

    ProgressBar::new(iterations).with_style(style)
}
