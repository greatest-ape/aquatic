use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;

use crate::set::TrackerCoreCountResults;

pub fn html_best_results(results: &[TrackerCoreCountResults]) -> String {
    let mut all_implementation_names = IndexSet::new();

    for core_count_results in results {
        all_implementation_names.extend(
            core_count_results
                .implementations
                .iter()
                .map(|r| r.name.clone()),
        );
    }

    let mut data_rows = Vec::new();

    for core_count_results in results {
        let best_results = core_count_results
            .implementations
            .iter()
            .map(|implementation| (implementation.name.clone(), implementation.best_result()))
            .collect::<IndexMap<_, _>>();

        let best_results_for_all_implementations = all_implementation_names
            .iter()
            .map(|name| best_results.get(name).cloned().flatten())
            .collect::<Vec<_>>();

        let data_row = format!(
            "
            <tr>
                <th>{}</th>
                {}
            </tr>
            ",
            core_count_results.core_count,
            best_results_for_all_implementations
                .into_iter()
                .map(|result| {
                    if let Some(r) = result {
                        format!(
                            r#"<td><span title="{}, avg cpu utilization: {}%">{}</span></td>"#,
                            r.tracker_info,
                            r.tracker_process_stats.avg_cpu_utilization,
                            r.average_responses,
                        )
                    } else {
                        "<td>-</td>".to_string()
                    }
                })
                .join("\n"),
        );

        data_rows.push(data_row);
    }

    format!(
        "
        <table>
            <thead>
                <tr>
                    <th>CPU cores</th>
                    {}
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
        ",
        all_implementation_names
            .iter()
            .map(|name| format!("<th>{name}</th>"))
            .join("\n"),
        data_rows.join("\n")
    )
}
