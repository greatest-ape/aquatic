use rand::Rng;
use rand_distr::Pareto;


pub fn pareto_usize(
    rng: &mut impl Rng,
    pareto: Pareto<f64>,
    max: usize,
) -> usize {
    let p: f64 = rng.sample(pareto);
    let p = (p.min(101.0f64) - 1.0) / 100.0;

    (p * max as f64) as usize
}