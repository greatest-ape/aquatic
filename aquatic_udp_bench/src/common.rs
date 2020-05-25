use indicatif::{ProgressBar, ProgressStyle};
use rand::Rng;
use rand_distr::Pareto;


pub const PARETO_SHAPE: f64 = 0.1;
pub const NUM_INFO_HASHES: usize = 10_000;


pub fn create_progress_bar(name: &str, iterations: u64) -> ProgressBar {
    let t = format!("{:<8} {}", name, "{wide_bar} {pos:>2}/{len:>2}");
    let style = ProgressStyle::default_bar().template(&t);

    ProgressBar::new(iterations).with_style(style)
}


pub fn pareto_usize(
    rng: &mut impl Rng,
    pareto: Pareto<f64>,
    max: usize,
) -> usize {
    let p: f64 = rng.sample(pareto);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}