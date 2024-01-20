use humanize_bytes::humanize_bytes_binary;
use indexmap::{IndexMap, IndexSet};
use indoc::formatdoc;
use itertools::Itertools;
use num_format::{Locale, ToFormattedString};

use crate::{
    run::ProcessStats,
    set::{LoadTestRunResults, TrackerCoreCountResults},
};

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
                            r.average_responses.to_formatted_string(&Locale::en),
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
        <h2>Best results</h2>
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

pub fn html_all_runs(all_results: &[TrackerCoreCountResults]) -> String {
    let mut all_implementation_names = IndexSet::new();

    for core_count_results in all_results {
        all_implementation_names.extend(
            core_count_results
                .implementations
                .iter()
                .map(|r| r.name.clone()),
        );
    }

    struct R {
        core_count: usize,
        avg_responses: Option<u64>,
        tracker_keys: IndexMap<String, String>,
        tracker_vcpus: String,
        tracker_stats: Option<ProcessStats>,
        load_test_keys: IndexMap<String, String>,
        load_test_vcpus: String,
    }

    let mut output = String::new();

    let mut results_by_implementation: IndexMap<String, Vec<R>> = Default::default();

    for implementation_name in all_implementation_names {
        let results = results_by_implementation
            .entry(implementation_name.clone())
            .or_default();

        let mut tracker_key_names: IndexSet<String> = Default::default();
        let mut load_test_key_names: IndexSet<String> = Default::default();

        for r in all_results {
            for i in r
                .implementations
                .iter()
                .filter(|i| i.name == implementation_name)
            {
                for c in i.configurations.iter() {
                    for l in c.load_tests.iter() {
                        match l {
                            LoadTestRunResults::Success(l) => {
                                tracker_key_names.extend(l.tracker_keys.keys().cloned());
                                load_test_key_names.extend(l.load_test_keys.keys().cloned());

                                results.push(R {
                                    core_count: r.core_count,
                                    avg_responses: Some(l.average_responses),
                                    tracker_keys: l.tracker_keys.clone(),
                                    tracker_vcpus: l.tracker_vcpus.as_cpu_list(),
                                    tracker_stats: Some(l.tracker_process_stats),
                                    load_test_keys: l.load_test_keys.clone(),
                                    load_test_vcpus: l.load_test_vcpus.as_cpu_list(),
                                })
                            }
                            LoadTestRunResults::Failure(l) => {
                                tracker_key_names.extend(l.tracker_keys.keys().cloned());
                                load_test_key_names.extend(l.load_test_keys.keys().cloned());

                                results.push(R {
                                    core_count: r.core_count,
                                    avg_responses: None,
                                    tracker_keys: l.tracker_keys.clone(),
                                    tracker_vcpus: l.tracker_vcpus.as_cpu_list(),
                                    tracker_stats: None,
                                    load_test_keys: l.load_test_keys.clone(),
                                    load_test_vcpus: l.load_test_vcpus.as_cpu_list(),
                                })
                            }
                        }
                    }
                }
            }
        }

        output.push_str(&formatdoc! {
            "
            <h2>Results for {implementation}</h2>
            <table>
                <thead>
                    <tr>
                        <th>Cores</th>
                        <th>Responses</th>
                        {tracker_key_names}
                        <th>Tracker avg CPU</th>
                        <th>Tracker peak RSS</th>
                        <th>Tracker vCPUs</th>
                        {load_test_key_names}
                        <th>Load test vCPUs</th>
                    </tr>
                </thead>
                <tbody>
                {body}
                </tbody>
            </table>
            ",
            implementation = implementation_name,
            tracker_key_names = tracker_key_names.iter()
                .map(|name| format!("<th>{}</th>", name))
                .join("\n"),
            load_test_key_names = load_test_key_names.iter()
                .map(|name| format!("<th>Load test {}</th>", name))
                .join("\n"),
            body = results.iter_mut().map(|r| {
                formatdoc! {
                    "
                    <tr>
                        <td>{cores}</td>
                        <td>{avg_responses}</td>
                        {tracker_key_values}
                        <td>{cpu}%</td>
                        <td>{mem}</td>
                        <td>{tracker_vcpus}</td>
                        {load_test_key_values}
                        <td>{load_test_vcpus}</td>
                    </tr>
                    ",
                    cores = r.core_count,
                    avg_responses = r.avg_responses.map(|v| v.to_formatted_string(&Locale::en))
                        .unwrap_or_else(|| "-".to_string()),
                    tracker_key_values = tracker_key_names.iter().map(|name| {
                        format!("<td>{}</td>", r.tracker_keys.get(name).cloned().unwrap_or_else(|| "-".to_string()))
                    }).join("\n"),
                    cpu = r.tracker_stats.map(|stats| stats.avg_cpu_utilization.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    mem = r.tracker_stats
                        .map(|stats| humanize_bytes_binary!(stats.peak_rss_bytes).to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    tracker_vcpus = r.tracker_vcpus,
                    load_test_key_values = load_test_key_names.iter().map(|name| {
                        format!("<td>{}</td>", r.load_test_keys.get(name).cloned().unwrap_or_else(|| "-".to_string()))
                    }).join("\n"),
                    load_test_vcpus = r.load_test_vcpus,
                }
            }).join("\n")
        });
    }

    output
}
