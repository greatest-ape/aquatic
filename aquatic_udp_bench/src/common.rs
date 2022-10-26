use indicatif::{ProgressBar, ProgressStyle};
use rand::Rng;
use rand_distr::Gamma;

pub const GAMMA_SHAPE: f64 = 0.2;
pub const GAMMA_SCALE: f64 = 100.0;

pub const NUM_INFO_HASHES: usize = 10_000;

pub fn create_progress_bar(name: &str, iterations: u64) -> ProgressBar {
    let t = format!("{:<8} {}", name, "{wide_bar} {pos:>2}/{len:>2}");
    let style = ProgressStyle::default_bar().template(&t);

    ProgressBar::new(iterations).with_style(style)
}

pub fn gamma_usize(rng: &mut impl Rng, gamma: Gamma<f64>, max: usize) -> usize {
    let p: f64 = rng.sample(gamma);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}
