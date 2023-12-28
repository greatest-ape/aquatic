use std::rc::Rc;

use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;

use crate::{
    common::{CpuDirection, CpuMode, TaskSetCpuList},
    run::{ProcessRunner, ProcessStats, RunConfig},
};

pub trait Tracker: ::std::fmt::Debug + Copy + Clone + ::std::hash::Hash {
    fn name(&self) -> String;
}

pub struct SetConfig<C, I> {
    pub implementations: IndexMap<I, Vec<Rc<dyn ProcessRunner<Command = C>>>>,
    pub load_test_runs: Vec<(usize, TaskSetCpuList)>,
}

pub fn run_sets<C, F, I>(
    command: &C,
    cpu_mode: CpuMode,
    set_configs: IndexMap<usize, SetConfig<C, I>>,
    load_test_gen: F,
) where
    C: ::std::fmt::Debug,
    I: Tracker,
    F: Fn(usize) -> Box<dyn ProcessRunner<Command = C>>,
{
    println!("# Benchmark report");

    let total_num_runs = set_configs
        .values()
        .map(|set| {
            set.implementations.values().map(Vec::len).sum::<usize>() * set.load_test_runs.len()
        })
        .sum::<usize>();

    let (estimated_hours, estimated_minutes) = {
        let minutes = (total_num_runs * 67) / 60;

        (minutes / 60, minutes % 60)
    };

    println!("");
    println!("Total number of load test runs: {}", total_num_runs);
    println!(
        "Estimated duration: {} hours, {} minutes",
        estimated_hours, estimated_minutes
    );
    println!("");

    let results = set_configs
        .into_iter()
        .map(|(tracker_core_count, set_config)| {
            let tracker_vcpus =
                TaskSetCpuList::new(cpu_mode, CpuDirection::Asc, tracker_core_count).unwrap();

            println!(
                "## Tracker cores: {} (cpus: {})",
                tracker_core_count,
                tracker_vcpus.as_cpu_list()
            );

            let tracker_results = set_config
                .implementations
                .into_iter()
                .map(|(implementation, tracker_runs)| {
                    let tracker_run_results = tracker_runs
                        .iter()
                        .map(|tracker_run| {
                            let load_test_run_results = set_config
                                .load_test_runs
                                .clone()
                                .into_iter()
                                .map(|(workers, load_test_vcpus)| {
                                    LoadTestRunResults::produce(
                                        command,
                                        &load_test_gen,
                                        implementation,
                                        &tracker_run,
                                        tracker_vcpus.clone(),
                                        workers,
                                        load_test_vcpus,
                                    )
                                })
                                .collect();

                            TrackerConfigurationResults {
                                load_tests: load_test_run_results,
                            }
                        })
                        .collect();

                    ImplementationResults {
                        name: implementation.name(),
                        configurations: tracker_run_results,
                    }
                })
                .collect();

            TrackerCoreCountResults {
                core_count: tracker_core_count,
                implementations: tracker_results,
            }
        })
        .collect::<Vec<_>>();

    html_summary(&results);
}

pub struct TrackerCoreCountResults {
    core_count: usize,
    implementations: Vec<ImplementationResults>,
}

pub struct ImplementationResults {
    name: String,
    configurations: Vec<TrackerConfigurationResults>,
}

impl ImplementationResults {
    fn best_result(&self) -> Option<LoadTestRunResultsSuccess> {
        self.configurations
            .iter()
            .filter_map(|c| c.best_result())
            .reduce(|acc, r| {
                if r.average_responses > acc.average_responses {
                    r
                } else {
                    acc
                }
            })
    }
}

pub struct TrackerConfigurationResults {
    load_tests: Vec<LoadTestRunResults>,
}

impl TrackerConfigurationResults {
    fn best_result(&self) -> Option<LoadTestRunResultsSuccess> {
        self.load_tests
            .iter()
            .filter_map(|r| match r {
                LoadTestRunResults::Success(r) => Some(r.clone()),
                LoadTestRunResults::Failure(_) => None,
            })
            .reduce(|acc, r| {
                if r.average_responses > acc.average_responses {
                    r
                } else {
                    acc
                }
            })
    }
}

pub enum LoadTestRunResults {
    Success(LoadTestRunResultsSuccess),
    Failure(LoadTestRunResultsFailure),
}

impl LoadTestRunResults {
    pub fn produce<C, F, I>(
        command: &C,
        load_test_gen: &F,
        implementation: I,
        tracker_process: &Rc<dyn ProcessRunner<Command = C>>,
        tracker_vcpus: TaskSetCpuList,
        workers: usize,
        load_test_vcpus: TaskSetCpuList,
    ) -> Self
    where
        C: ::std::fmt::Debug,
        I: Tracker,
        F: Fn(usize) -> Box<dyn ProcessRunner<Command = C>>,
    {
        println!(
            "### {} run ({}) (load test workers: {}, cpus: {})",
            implementation.name(),
            tracker_process.info(),
            workers,
            load_test_vcpus.as_cpu_list()
        );

        let load_test_runner = load_test_gen(workers);
        // let load_test_keys = load_test_runner.keys();

        let run_config = RunConfig {
            tracker_runner: tracker_process.clone(),
            tracker_vcpus: tracker_vcpus.clone(),
            load_test_runner,
            load_test_vcpus,
        };

        match run_config.run(command) {
            Ok(r) => {
                println!("- Average responses per second: {}", r.avg_responses);
                println!(
                    "- Average tracker CPU utilization: {}%",
                    r.tracker_process_stats.avg_cpu_utilization,
                );
                println!(
                    "- Peak tracker RSS: {} kB",
                    r.tracker_process_stats.peak_rss_kb
                );

                LoadTestRunResults::Success(LoadTestRunResultsSuccess {
                    average_responses: r.avg_responses,
                    // tracker_keys: tracker_process.keys(),
                    tracker_info: tracker_process.info(),
                    tracker_process_stats: r.tracker_process_stats,
                    // load_test_keys,
                })
            }
            Err(results) => {
                println!("\nRun failed:\n{:#}\n", results);

                LoadTestRunResults::Failure(LoadTestRunResultsFailure {
                    // load_test_keys
                })
            }
        }
    }
}

#[derive(Clone)]
pub struct LoadTestRunResultsSuccess {
    average_responses: f32,
    // tracker_keys: IndexMap<String, String>,
    tracker_info: String,
    tracker_process_stats: ProcessStats,
    // load_test_keys: IndexMap<String, String>,
}

pub struct LoadTestRunResultsFailure {
    // load_test_keys: IndexMap<String, String>,
}

pub fn html_summary(results: &[TrackerCoreCountResults]) {
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

    println!(
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
