use plotly::{Plot, Scatter, Layout};
use plotly::common::Title;
use plotly::layout::Axis;
use rand::{thread_rng, rngs::SmallRng, SeedableRng};
use rand_distr::Pareto;

use aquatic::bench_utils::pareto_usize;


fn main(){
    let mut plot = Plot::new();
    let mut rng = SmallRng::from_rng(thread_rng()).unwrap();

    const LEN: usize = 1_000;
    const MAX_VAL: usize = LEN - 1;

    for pareto_shape in [0.1, 0.2, 0.3, 0.4, 0.5].iter() {
        let pareto = Pareto::new(1.0, *pareto_shape).unwrap();

        let mut y_axis = [0; LEN];

        for _ in 1..1_000_000 {
            let index = pareto_usize(&mut rng, pareto, MAX_VAL);

            y_axis[index] += 1;
        }

        let x_axis: Vec<usize> = (0..MAX_VAL).into_iter().collect();

        let trace = Scatter::new(x_axis, y_axis.to_vec())
            .name(&format!("pareto shape = {}", pareto_shape));

        plot.add_trace(trace);
    }

    let layout = Layout::new()
        .title(Title::new("Pareto distribution"))
        .xaxis(Axis::new().title(Title::new("Info hash index")))
        .yaxis(Axis::new().title(Title::new("Num requests")));
    
    plot.set_layout(layout);

    plot.show();
}
