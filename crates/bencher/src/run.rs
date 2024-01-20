use std::{
    process::{Child, Command},
    rc::Rc,
    str::FromStr,
    time::Duration,
};

use indexmap::IndexMap;
use itertools::Itertools;
use nonblock::NonBlockingReader;
use once_cell::sync::Lazy;
use regex::Regex;
use tempfile::NamedTempFile;

use crate::common::{Priority, TaskSetCpuList};

pub trait ProcessRunner: ::std::fmt::Debug {
    type Command;

    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child>;

    fn keys(&self) -> IndexMap<String, String>;

    fn priority(&self) -> Priority;

    fn info(&self) -> String {
        self.keys()
            .into_iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .join(", ")
    }
}

#[derive(Debug)]
pub struct RunConfig<C> {
    pub tracker_runner: Rc<dyn ProcessRunner<Command = C>>,
    pub tracker_vcpus: TaskSetCpuList,
    pub load_test_runner: Box<dyn ProcessRunner<Command = C>>,
    pub load_test_vcpus: TaskSetCpuList,
}

impl<C> RunConfig<C> {
    pub fn run(
        self,
        command: &C,
        duration: usize,
    ) -> Result<RunSuccessResults, RunErrorResults<C>> {
        let mut tracker_config_file = NamedTempFile::new().unwrap();
        let mut load_test_config_file = NamedTempFile::new().unwrap();

        let mut tracker =
            match self
                .tracker_runner
                .run(command, &self.tracker_vcpus, &mut tracker_config_file)
            {
                Ok(handle) => ChildWrapper(handle),
                Err(err) => return Err(RunErrorResults::new(self).set_error(err, "run tracker")),
            };

        ::std::thread::sleep(Duration::from_secs(1));

        let mut load_tester = match self.load_test_runner.run(
            command,
            &self.load_test_vcpus,
            &mut load_test_config_file,
        ) {
            Ok(handle) => ChildWrapper(handle),
            Err(err) => {
                return Err(RunErrorResults::new(self)
                    .set_error(err, "run load test")
                    .set_tracker_outputs(tracker))
            }
        };

        for _ in 0..(duration - 1) {
            if let Ok(Some(status)) = tracker.0.try_wait() {
                return Err(RunErrorResults::new(self)
                    .set_tracker_outputs(tracker)
                    .set_load_test_outputs(load_tester)
                    .set_error_context(&format!("tracker exited with {}", status)));
            }

            ::std::thread::sleep(Duration::from_secs(1));
        }

        // Note: a more advanced version tracking threads too would add argument
        // "-L" and add "comm" to output format list
        let tracker_process_stats_res = Command::new("ps")
            .arg("-p")
            .arg(tracker.0.id().to_string())
            .arg("-o")
            .arg("%cpu,rss")
            .arg("--noheader")
            .output();

        let tracker_process_stats = match tracker_process_stats_res {
            Ok(output) if output.status.success() => {
                ProcessStats::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap()
            }
            Ok(_) => {
                return Err(RunErrorResults::new(self)
                    .set_error_context("run ps")
                    .set_tracker_outputs(tracker)
                    .set_load_test_outputs(load_tester));
            }
            Err(err) => {
                return Err(RunErrorResults::new(self)
                    .set_error(err.into(), "run ps")
                    .set_tracker_outputs(tracker)
                    .set_load_test_outputs(load_tester));
            }
        };

        ::std::thread::sleep(Duration::from_secs(5));

        let (load_test_stdout, load_test_stderr) = match load_tester.0.try_wait() {
            Ok(Some(status)) if status.success() => read_child_outputs(load_tester),
            Ok(Some(_)) => {
                return Err(RunErrorResults::new(self)
                    .set_error_context("wait for load tester")
                    .set_tracker_outputs(tracker)
                    .set_load_test_outputs(load_tester))
            }
            Ok(None) => {
                if let Err(err) = load_tester.0.kill() {
                    return Err(RunErrorResults::new(self)
                        .set_error(err.into(), "kill load tester")
                        .set_tracker_outputs(tracker)
                        .set_load_test_outputs(load_tester));
                }

                ::std::thread::sleep(Duration::from_secs(1));

                match load_tester.0.try_wait() {
                    Ok(_) => {
                        return Err(RunErrorResults::new(self)
                            .set_error_context("load tester didn't finish in time")
                            .set_load_test_outputs(load_tester))
                    }
                    Err(err) => {
                        return Err(RunErrorResults::new(self)
                            .set_error(err.into(), "wait for load tester after kill")
                            .set_tracker_outputs(tracker));
                    }
                }
            }
            Err(err) => {
                return Err(RunErrorResults::new(self)
                    .set_error(err.into(), "wait for load tester")
                    .set_tracker_outputs(tracker)
                    .set_load_test_outputs(load_tester))
            }
        };

        let load_test_stdout = if let Some(load_test_stdout) = load_test_stdout {
            load_test_stdout
        } else {
            return Err(RunErrorResults::new(self)
                .set_error_context("couldn't read load tester stdout")
                .set_tracker_outputs(tracker)
                .set_load_test_stderr(load_test_stderr));
        };

        let avg_responses = {
            static RE: Lazy<Regex> =
                Lazy::new(|| Regex::new(r"Average responses per second: ([0-9]+)").unwrap());

            let opt_avg_responses = RE
                .captures_iter(&load_test_stdout)
                .next()
                .map(|c| {
                    let (_, [avg_responses]) = c.extract();

                    avg_responses.to_string()
                })
                .and_then(|v| v.parse::<u64>().ok());

            if let Some(avg_responses) = opt_avg_responses {
                avg_responses
            } else {
                return Err(RunErrorResults::new(self)
                    .set_error_context("couldn't extract avg_responses")
                    .set_tracker_outputs(tracker)
                    .set_load_test_stdout(Some(load_test_stdout))
                    .set_load_test_stderr(load_test_stderr));
            }
        };

        let results = RunSuccessResults {
            tracker_process_stats,
            avg_responses,
        };

        Ok(results)
    }
}

pub struct RunSuccessResults {
    pub tracker_process_stats: ProcessStats,
    pub avg_responses: u64,
}

#[derive(Debug)]
pub struct RunErrorResults<C> {
    pub run_config: RunConfig<C>,
    pub tracker_stdout: Option<String>,
    pub tracker_stderr: Option<String>,
    pub load_test_stdout: Option<String>,
    pub load_test_stderr: Option<String>,
    pub error: Option<anyhow::Error>,
    pub error_context: Option<String>,
}

impl<C> RunErrorResults<C> {
    fn new(run_config: RunConfig<C>) -> Self {
        Self {
            run_config,
            tracker_stdout: Default::default(),
            tracker_stderr: Default::default(),
            load_test_stdout: Default::default(),
            load_test_stderr: Default::default(),
            error: Default::default(),
            error_context: Default::default(),
        }
    }

    fn set_tracker_outputs(mut self, tracker: ChildWrapper) -> Self {
        let (stdout, stderr) = read_child_outputs(tracker);

        self.tracker_stdout = stdout;
        self.tracker_stderr = stderr;

        self
    }

    fn set_load_test_outputs(mut self, load_test: ChildWrapper) -> Self {
        let (stdout, stderr) = read_child_outputs(load_test);

        self.load_test_stdout = stdout;
        self.load_test_stderr = stderr;

        self
    }

    fn set_load_test_stdout(mut self, stdout: Option<String>) -> Self {
        self.load_test_stdout = stdout;

        self
    }

    fn set_load_test_stderr(mut self, stderr: Option<String>) -> Self {
        self.load_test_stderr = stderr;

        self
    }

    fn set_error(mut self, error: anyhow::Error, context: &str) -> Self {
        self.error = Some(error);
        self.error_context = Some(context.to_string());

        self
    }

    fn set_error_context(mut self, context: &str) -> Self {
        self.error_context = Some(context.to_string());

        self
    }
}

impl<C> std::fmt::Display for RunErrorResults<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(t) = self.error_context.as_ref() {
            writeln!(f, "- {}", t)?;
        }
        if let Some(err) = self.error.as_ref() {
            writeln!(f, "- {:#}", err)?;
        }

        writeln!(f, "- tracker_runner: {:?}", self.run_config.tracker_runner)?;
        writeln!(
            f,
            "- load_test_runner: {:?}",
            self.run_config.load_test_runner
        )?;
        writeln!(
            f,
            "- tracker_vcpus: {}",
            self.run_config.tracker_vcpus.as_cpu_list()
        )?;
        writeln!(
            f,
            "- load_test_vcpus: {}",
            self.run_config.load_test_vcpus.as_cpu_list()
        )?;

        if let Some(t) = self.tracker_stdout.as_ref() {
            writeln!(f, "- tracker stdout:\n```\n{}\n```", t)?;
        }
        if let Some(t) = self.tracker_stderr.as_ref() {
            writeln!(f, "- tracker stderr:\n```\n{}\n```", t)?;
        }
        if let Some(t) = self.load_test_stdout.as_ref() {
            writeln!(f, "- load test stdout:\n```\n{}\n```", t)?;
        }
        if let Some(t) = self.load_test_stderr.as_ref() {
            writeln!(f, "- load test stderr:\n```\n{}\n```", t)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessStats {
    pub avg_cpu_utilization: f32,
    pub peak_rss_bytes: u64,
}

impl FromStr for ProcessStats {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split_whitespace();

        let avg_cpu_utilization = parts.next().ok_or(())?.parse().map_err(|_| ())?;
        let peak_rss_kb: f32 = parts.next().ok_or(())?.parse().map_err(|_| ())?;

        Ok(Self {
            avg_cpu_utilization,
            peak_rss_bytes: (peak_rss_kb * 1000.0) as u64,
        })
    }
}

struct ChildWrapper(Child);

impl Drop for ChildWrapper {
    fn drop(&mut self) {
        let _ = self.0.kill();

        ::std::thread::sleep(Duration::from_secs(1));

        let _ = self.0.try_wait();
    }
}

fn read_child_outputs(mut child: ChildWrapper) -> (Option<String>, Option<String>) {
    let stdout = child.0.stdout.take().and_then(|stdout| {
        let mut buf = String::new();

        let mut reader = NonBlockingReader::from_fd(stdout).unwrap();

        reader.read_available_to_string(&mut buf).unwrap();

        (!buf.is_empty()).then_some(buf)
    });
    let stderr = child.0.stderr.take().and_then(|stderr| {
        let mut buf = String::new();

        let mut reader = NonBlockingReader::from_fd(stderr).unwrap();

        reader.read_available_to_string(&mut buf).unwrap();

        (!buf.is_empty()).then_some(buf)
    });

    (stdout, stderr)
}
