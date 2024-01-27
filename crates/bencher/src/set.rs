use std::rc::Rc;

use humanize_bytes::humanize_bytes_binary;
use indexmap::IndexMap;
use num_format::{Locale, ToFormattedString};

use crate::{
    common::{CpuDirection, CpuMode, Priority, TaskSetCpuList},
    html::{html_all_runs, html_best_results},
    run::{ProcessRunner, ProcessStats, RunConfig},
};

#[derive(Debug, Clone, Copy)]
pub struct LoadTestRunnerParameters {
    pub workers: usize,
    pub duration: usize,
    pub summarize_last: usize,
}

pub trait Tracker: ::std::fmt::Debug + Copy + Clone + ::std::hash::Hash {
    fn name(&self) -> String;
}

pub struct SetConfig<C, I> {
    pub implementations: IndexMap<I, Vec<Rc<dyn ProcessRunner<Command = C>>>>,
    pub load_test_runs: Vec<(usize, Priority, TaskSetCpuList)>,
}

#[allow(clippy::too_many_arguments)]
pub fn run_sets<C, F, I>(
    command: &C,
    cpu_mode: CpuMode,
    min_cores: Option<usize>,
    max_cores: Option<usize>,
    min_priority: Priority,
    duration: usize,
    summarize_last: usize,
    mut set_configs: IndexMap<usize, SetConfig<C, I>>,
    load_test_gen: F,
) where
    C: ::std::fmt::Debug,
    I: Tracker,
    F: Fn(LoadTestRunnerParameters) -> Box<dyn ProcessRunner<Command = C>>,
{
    if let Some(min_cores) = min_cores {
        set_configs.retain(|cores, _| *cores >= min_cores);
    }
    if let Some(max_cores) = max_cores {
        set_configs.retain(|cores, _| *cores <= max_cores);
    }

    for set_config in set_configs.values_mut() {
        for runners in set_config.implementations.values_mut() {
            runners.retain(|r| r.priority() >= min_priority);
        }

        set_config
            .load_test_runs
            .retain(|(_, priority, _)| *priority >= min_priority);
    }

    println!("# Benchmark report");

    let total_num_runs = set_configs
        .values()
        .map(|set| {
            set.implementations.values().map(Vec::len).sum::<usize>() * set.load_test_runs.len()
        })
        .sum::<usize>();

    let (estimated_hours, estimated_minutes) = {
        let minutes = (total_num_runs * (duration + 7)) / 60;

        (minutes / 60, minutes % 60)
    };

    println!();
    println!("Total number of load test runs: {}", total_num_runs);
    println!(
        "Estimated duration: {} hours, {} minutes",
        estimated_hours, estimated_minutes
    );
    println!();

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
                                .map(|(workers, _, load_test_vcpus)| {
                                    let load_test_parameters = LoadTestRunnerParameters {
                                        workers,
                                        duration,
                                        summarize_last,
                                    };
                                    LoadTestRunResults::produce(
                                        command,
                                        &load_test_gen,
                                        load_test_parameters,
                                        implementation,
                                        tracker_run,
                                        tracker_vcpus.clone(),
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

    println!("{}", html_all_runs(&results));
    println!("{}", html_best_results(&results));
}

pub struct TrackerCoreCountResults {
    pub core_count: usize,
    pub implementations: Vec<ImplementationResults>,
}

pub struct ImplementationResults {
    pub name: String,
    pub configurations: Vec<TrackerConfigurationResults>,
}

impl ImplementationResults {
    pub fn best_result(&self) -> Option<LoadTestRunResultsSuccess> {
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
    pub load_tests: Vec<LoadTestRunResults>,
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
        load_test_parameters: LoadTestRunnerParameters,
        implementation: I,
        tracker_process: &Rc<dyn ProcessRunner<Command = C>>,
        tracker_vcpus: TaskSetCpuList,
        load_test_vcpus: TaskSetCpuList,
    ) -> Self
    where
        C: ::std::fmt::Debug,
        I: Tracker,
        F: Fn(LoadTestRunnerParameters) -> Box<dyn ProcessRunner<Command = C>>,
    {
        println!(
            "### {} run ({}) (load test workers: {}, cpus: {})",
            implementation.name(),
            tracker_process.info(),
            load_test_parameters.workers,
            load_test_vcpus.as_cpu_list()
        );

        let load_test_runner = load_test_gen(load_test_parameters);
        let load_test_keys = load_test_runner.keys();

        let run_config = RunConfig {
            tracker_runner: tracker_process.clone(),
            tracker_vcpus: tracker_vcpus.clone(),
            load_test_runner,
            load_test_vcpus: load_test_vcpus.clone(),
        };

        match run_config.run(command, load_test_parameters.duration) {
            Ok(r) => {
                println!(
                    "- Average responses per second: {}",
                    r.avg_responses.to_formatted_string(&Locale::en)
                );
                println!(
                    "- Average tracker CPU utilization: {}%",
                    r.tracker_process_stats.avg_cpu_utilization,
                );
                println!(
                    "- Peak tracker RSS: {}",
                    humanize_bytes_binary!(r.tracker_process_stats.peak_rss_bytes)
                );

                LoadTestRunResults::Success(LoadTestRunResultsSuccess {
                    average_responses: r.avg_responses,
                    tracker_keys: tracker_process.keys(),
                    tracker_info: tracker_process.info(),
                    tracker_process_stats: r.tracker_process_stats,
                    tracker_vcpus,
                    load_test_keys,
                    load_test_vcpus,
                })
            }
            Err(results) => {
                println!("\nRun failed:\n{:#}\n", results);

                LoadTestRunResults::Failure(LoadTestRunResultsFailure {
                    tracker_keys: tracker_process.keys(),
                    tracker_vcpus,
                    load_test_keys,
                    load_test_vcpus,
                })
            }
        }
    }
}

#[derive(Clone)]
pub struct LoadTestRunResultsSuccess {
    pub average_responses: u64,
    pub tracker_keys: IndexMap<String, String>,
    pub tracker_info: String,
    pub tracker_process_stats: ProcessStats,
    pub tracker_vcpus: TaskSetCpuList,
    pub load_test_keys: IndexMap<String, String>,
    pub load_test_vcpus: TaskSetCpuList,
}

pub struct LoadTestRunResultsFailure {
    pub tracker_keys: IndexMap<String, String>,
    pub tracker_vcpus: TaskSetCpuList,
    pub load_test_keys: IndexMap<String, String>,
    pub load_test_vcpus: TaskSetCpuList,
}
