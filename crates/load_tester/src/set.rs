use std::rc::Rc;

use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;

use crate::{
    common::{CpuDirection, CpuMode, TaskSetCpuList},
    run::{ProcessRunner, ProcessStats, RunConfig},
};

pub trait Server: ::std::fmt::Debug + Copy + Clone + ::std::hash::Hash {
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
    I: Server,
    F: Fn(usize) -> Box<dyn ProcessRunner<Command = C>>,
{
    println!("# Load test report");

    let results = set_configs
        .into_iter()
        .map(|(server_core_count, set_config)| {
            let server_vcpus =
                TaskSetCpuList::new(cpu_mode, CpuDirection::Asc, server_core_count).unwrap();

            println!(
                "## Tracker cores: {} (cpus: {})",
                server_core_count,
                server_vcpus.as_cpu_list()
            );

            let server_results = set_config
                .implementations
                .into_iter()
                .map(|(implementation, server_runs)| {
                    let server_run_results = server_runs
                        .iter()
                        .map(|server_run| {
                            let load_test_run_results = set_config
                                .load_test_runs
                                .clone()
                                .into_iter()
                                .map(|(workers, load_test_vcpus)| {
                                    LoadTestRunResults::produce(
                                        command,
                                        &load_test_gen,
                                        implementation,
                                        &server_run,
                                        server_vcpus.clone(),
                                        workers,
                                        load_test_vcpus,
                                    )
                                })
                                .collect();

                            ServerConfigurationResults {
                                config_keys: server_run.keys(),
                                load_tests: load_test_run_results,
                            }
                        })
                        .collect();

                    ImplementationResults {
                        name: implementation.name(),
                        configurations: server_run_results,
                    }
                })
                .collect();

            ServerCoreCountResults {
                core_count: server_core_count,
                implementations: server_results,
            }
        })
        .collect::<Vec<_>>();

    html_summary(&results);
}

pub struct ServerCoreCountResults {
    core_count: usize,
    implementations: Vec<ImplementationResults>,
}

pub struct ImplementationResults {
    name: String,
    configurations: Vec<ServerConfigurationResults>,
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

pub struct ServerConfigurationResults {
    config_keys: IndexMap<String, String>,
    load_tests: Vec<LoadTestRunResults>,
    // best_index: Option<usize>,
}

impl ServerConfigurationResults {
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
        server_process: &Rc<dyn ProcessRunner<Command = C>>,
        server_vcpus: TaskSetCpuList,
        workers: usize,
        load_test_vcpus: TaskSetCpuList,
    ) -> Self
    where
        C: ::std::fmt::Debug,
        I: Server,
        F: Fn(usize) -> Box<dyn ProcessRunner<Command = C>>,
    {
        println!(
            "### {} run ({}) (load test workers: {}, cpus: {})",
            implementation.name(),
            server_process.info(),
            workers,
            load_test_vcpus.as_cpu_list()
        );

        let load_test_runner = load_test_gen(workers);
        let load_test_keys = load_test_runner.keys();

        let run_config = RunConfig {
            server_runner: server_process.clone(),
            server_vcpus: server_vcpus.clone(),
            load_test_runner,
            load_test_vcpus,
        };

        match run_config.run(command) {
            Ok(results) => {
                let avg_responses = results.avg_responses().unwrap().parse::<f32>().unwrap();
                let server_process_stats = results.server_process_stats.unwrap();

                println!("- Average responses per second: {}", avg_responses);
                println!(
                    "- Average server CPU utilization: {}%",
                    server_process_stats.avg_cpu_utilization,
                );
                println!("- Peak server RSS: {} kB", server_process_stats.peak_rss_kb);

                LoadTestRunResults::Success(LoadTestRunResultsSuccess {
                    config_keys: load_test_keys,
                    average_responses: avg_responses,
                    server_process_stats,
                })
            }
            Err(results) => {
                println!("\nRun failed:\n{:?}\n", results);

                LoadTestRunResults::Failure(LoadTestRunResultsFailure {
                    config_keys: load_test_keys,
                })
            }
        }
    }
}

#[derive(Clone)]
pub struct LoadTestRunResultsSuccess {
    config_keys: IndexMap<String, String>,
    average_responses: f32,
    server_process_stats: ProcessStats,
}

pub struct LoadTestRunResultsFailure {
    config_keys: IndexMap<String, String>,
}

pub fn html_summary(results: &[ServerCoreCountResults]) {
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
            .map(|name| {
                best_results
                    .get(name)
                    .and_then(|r| r.as_ref().map(|r| r.average_responses))
            })
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
                .map(|result| format!(
                    "<td>{}</td>",
                    result
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "-".to_string())
                ))
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
