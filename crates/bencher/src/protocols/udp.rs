use std::{
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
};

use clap::Parser;
use indexmap::{indexmap, IndexMap};
use indoc::writedoc;
use tempfile::NamedTempFile;

use crate::{
    common::{simple_load_test_runs, CpuMode, TaskSetCpuList},
    run::ProcessRunner,
    set::{run_sets, SetConfig, Tracker},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UdpTracker {
    Aquatic,
    OpenTracker,
    Chihaya,
}

impl Tracker for UdpTracker {
    fn name(&self) -> String {
        match self {
            Self::Aquatic => "aquatic_udp".into(),
            Self::OpenTracker => "opentracker".into(),
            Self::Chihaya => "chihaya".into(),
        }
    }
}

#[derive(Parser, Debug)]
pub struct UdpCommand {
    /// Path to aquatic_udp_load_test binary
    #[arg(long, default_value = "./target/release-debug/aquatic_udp_load_test")]
    load_test: PathBuf,
    /// Path to aquatic_udp binary
    #[arg(long, default_value = "./target/release-debug/aquatic_udp")]
    aquatic: PathBuf,
    /// Path to opentracker binary
    #[arg(long, default_value = "opentracker")]
    opentracker: PathBuf,
    /// Path to chihaya binary
    #[arg(long, default_value = "chihaya")]
    chihaya: PathBuf,
}

impl UdpCommand {
    pub fn run(
        &self,
        cpu_mode: CpuMode,
        min_cores: Option<usize>,
        max_cores: Option<usize>,
    ) -> anyhow::Result<()> {
        let mut sets = self.sets(cpu_mode);

        if let Some(min_cores) = min_cores {
            sets = sets.into_iter().filter(|(k, _)| *k >= min_cores).collect();
        }
        if let Some(max_cores) = max_cores {
            sets = sets.into_iter().filter(|(k, _)| *k <= max_cores).collect();
        }

        run_sets(self, cpu_mode, sets, |workers| {
            Box::new(AquaticUdpLoadTestRunner { workers })
        });

        Ok(())
    }

    fn sets(&self, cpu_mode: CpuMode) -> IndexMap<usize, SetConfig<UdpCommand, UdpTracker>> {
        indexmap::indexmap! {
            1 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(1, 1),
                        AquaticUdpRunner::new(2, 1),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(0), // Handle requests within event loop
                        OpenTrackerUdpRunner::new(1),
                        OpenTrackerUdpRunner::new(2),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[1, 2, 4, 6, 8]),
            },
            2 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(1, 1),
                        AquaticUdpRunner::new(2, 1),
                        AquaticUdpRunner::new(3, 1),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(2),
                        OpenTrackerUdpRunner::new(4),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[1, 2, 4, 6, 8]),
            },
            3 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(2, 1),
                        AquaticUdpRunner::new(3, 1),
                        AquaticUdpRunner::new(4, 1),
                        AquaticUdpRunner::new(5, 1),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(3),
                        OpenTrackerUdpRunner::new(6),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[4, 6, 8, 12, 16]),
            },
            4 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(3, 1),
                        AquaticUdpRunner::new(4, 1),
                        AquaticUdpRunner::new(5, 1),
                        AquaticUdpRunner::new(6, 1),
                        AquaticUdpRunner::new(7, 1),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(4),
                        OpenTrackerUdpRunner::new(8),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[6, 8, 12, 16]),
            },
            6 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(5, 1),
                        AquaticUdpRunner::new(6, 1),
                        AquaticUdpRunner::new(10, 1),

                        AquaticUdpRunner::new(4, 2),
                        AquaticUdpRunner::new(6, 2),
                        AquaticUdpRunner::new(8, 2),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(6),
                        OpenTrackerUdpRunner::new(12),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[6, 8, 12, 16, 24]),
            },
            8 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(7, 1),
                        AquaticUdpRunner::new(8, 1),
                        AquaticUdpRunner::new(14, 1),
                        AquaticUdpRunner::new(6, 2),
                        AquaticUdpRunner::new(12, 2),
                        AquaticUdpRunner::new(5, 3),
                        AquaticUdpRunner::new(10, 3),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(8),
                        OpenTrackerUdpRunner::new(16),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[8, 12, 16, 24]),
            },
            12 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(10, 2),
                        AquaticUdpRunner::new(12, 2),
                        AquaticUdpRunner::new(20, 2),

                        AquaticUdpRunner::new(9, 3),
                        AquaticUdpRunner::new(12, 3),
                        AquaticUdpRunner::new(18, 3),

                        AquaticUdpRunner::new(8, 4),
                        AquaticUdpRunner::new(12, 4),
                        AquaticUdpRunner::new(16, 4),

                        AquaticUdpRunner::new(7, 5),
                        AquaticUdpRunner::new(12, 5),
                        AquaticUdpRunner::new(14, 5),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(12),
                        OpenTrackerUdpRunner::new(24),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[8, 12, 16, 24]),
            },
            16 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(12, 2),
                        AquaticUdpRunner::new(16, 2),
                        AquaticUdpRunner::new(24, 2),

                        AquaticUdpRunner::new(13, 3),
                        AquaticUdpRunner::new(16, 3),
                        AquaticUdpRunner::new(26, 3),

                        AquaticUdpRunner::new(12, 4),
                        AquaticUdpRunner::new(16, 4),
                        AquaticUdpRunner::new(24, 4),

                        AquaticUdpRunner::new(11, 5),
                        AquaticUdpRunner::new(16, 5),
                        AquaticUdpRunner::new(22, 5),

                        AquaticUdpRunner::new(10, 6),
                        AquaticUdpRunner::new(16, 6),
                        AquaticUdpRunner::new(20, 6),

                        AquaticUdpRunner::new(9, 7),
                        AquaticUdpRunner::new(16, 7),
                        AquaticUdpRunner::new(18, 7),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(16),
                        OpenTrackerUdpRunner::new(32),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[8, 12, 16, 24]),
            },
        }
    }
}

#[derive(Debug, Clone)]
struct AquaticUdpRunner {
    socket_workers: usize,
    swarm_workers: usize,
}

impl AquaticUdpRunner {
    fn new(
        socket_workers: usize,
        swarm_workers: usize,
    ) -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self {
            socket_workers,
            swarm_workers,
        })
    }
}

impl ProcessRunner for AquaticUdpRunner {
    type Command = UdpCommand;

    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child> {
        let mut c = aquatic_udp::config::Config::default();

        c.socket_workers = self.socket_workers;
        c.swarm_workers = self.swarm_workers;
        c.protocol.max_response_peers = 50;

        let c = toml::to_string_pretty(&c)?;

        tmp_file.write_all(c.as_bytes())?;

        Ok(Command::new("taskset")
            .arg("--cpu-list")
            .arg(vcpus.as_cpu_list())
            .arg(&command.aquatic)
            .arg("-c")
            .arg(tmp_file.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    }

    fn keys(&self) -> IndexMap<String, String> {
        indexmap! {
            "socket workers".to_string() => self.socket_workers.to_string(),
            "swarm workers".to_string() => self.swarm_workers.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct OpenTrackerUdpRunner {
    workers: usize,
}

impl OpenTrackerUdpRunner {
    fn new(workers: usize) -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self { workers })
    }
}

impl ProcessRunner for OpenTrackerUdpRunner {
    type Command = UdpCommand;

    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child> {
        writeln!(
            tmp_file,
            "listen.udp.workers {}\nlisten.udp 0.0.0.0:3000",
            self.workers
        )?;

        Ok(Command::new("taskset")
            .arg("--cpu-list")
            .arg(vcpus.as_cpu_list())
            .arg(&command.opentracker)
            .arg("-f")
            .arg(tmp_file.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    }

    fn keys(&self) -> IndexMap<String, String> {
        indexmap! {
            "workers".to_string() => self.workers.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct ChihayaUdpRunner;

impl ChihayaUdpRunner {
    fn new() -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self)
    }
}

impl ProcessRunner for ChihayaUdpRunner {
    type Command = UdpCommand;

    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child> {
        writedoc!(
            tmp_file,
            r#"
            ---
            chihaya:
              metrics_addr: "127.0.0.1:0"
              udp:
                addr: "127.0.0.1:3000"
                private_key: "abcdefghijklmnopqrst"
                max_numwant: 50
                default_numwant: 50
              storage:
                name: "memory"
            "#,
        )?;

        Ok(Command::new("taskset")
            .arg("--cpu-list")
            .arg(vcpus.as_cpu_list())
            .arg(&command.chihaya)
            .arg("--config")
            .arg(tmp_file.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    }

    fn keys(&self) -> IndexMap<String, String> {
        Default::default()
    }
}

#[derive(Debug, Clone)]
struct AquaticUdpLoadTestRunner {
    workers: usize,
}

impl ProcessRunner for AquaticUdpLoadTestRunner {
    type Command = UdpCommand;

    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child> {
        let mut c = aquatic_udp_load_test::config::Config::default();

        c.workers = self.workers as u8;
        c.duration = 60;

        c.requests.weight_connect = 0;
        c.requests.weight_announce = 100;
        c.requests.weight_scrape = 1;

        let c = toml::to_string_pretty(&c)?;

        tmp_file.write_all(c.as_bytes())?;

        Ok(Command::new("taskset")
            .arg("--cpu-list")
            .arg(vcpus.as_cpu_list())
            .arg(&command.load_test)
            .arg("-c")
            .arg(tmp_file.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    }

    fn keys(&self) -> IndexMap<String, String> {
        indexmap! {
            "workers".to_string() => self.workers.to_string(),
        }
    }
}
