use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
};

use clap::Parser;
use indexmap::{indexmap, IndexMap};
use indoc::writedoc;
use tempfile::NamedTempFile;

use crate::{
    common::{simple_load_test_runs, CpuMode, Priority, TaskSetCpuList},
    run::ProcessRunner,
    set::{LoadTestRunnerParameters, SetConfig, Tracker},
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
    pub fn sets(&self, cpu_mode: CpuMode) -> IndexMap<usize, SetConfig<UdpCommand, UdpTracker>> {
        // Priorities are based on what has previously produced the best results
        indexmap::indexmap! {
            1 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(1, 1, Priority::High),
                        AquaticUdpRunner::new(2, 1, Priority::High),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(0, Priority::Low), // Handle requests within event loop
                        OpenTrackerUdpRunner::new(1, Priority::Medium),
                        OpenTrackerUdpRunner::new(2, Priority::High),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (1, Priority::High),
                    (2, Priority::Medium),
                    (4, Priority::Medium),
                    (6, Priority::Medium),
                    (8, Priority::High)
                ]),
            },
            2 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(1, 1, Priority::Low),
                        AquaticUdpRunner::new(2, 1, Priority::Medium),
                        AquaticUdpRunner::new(3, 1, Priority::High),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(2, Priority::Medium),
                        OpenTrackerUdpRunner::new(4, Priority::High),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (1, Priority::Medium),
                    (2, Priority::Medium),
                    (4, Priority::Medium),
                    (6, Priority::Medium),
                    (8, Priority::High)
                ]),
            },
            4 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(3, 1, Priority::Low),
                        AquaticUdpRunner::new(4, 1, Priority::Low),
                        AquaticUdpRunner::new(5, 1, Priority::Medium),
                        AquaticUdpRunner::new(6, 1, Priority::Medium),
                        AquaticUdpRunner::new(7, 1, Priority::High),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(4, Priority::High),
                        OpenTrackerUdpRunner::new(8, Priority::Medium),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (6, Priority::High),
                    (8, Priority::Medium),
                    (12, Priority::High),
                    (16, Priority::Medium)
                ]),
            },
            6 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(5, 1, Priority::Medium),
                        AquaticUdpRunner::new(6, 1, Priority::Medium),
                        AquaticUdpRunner::new(10, 1, Priority::Low),

                        AquaticUdpRunner::new(4, 2, Priority::Low),
                        AquaticUdpRunner::new(6, 2, Priority::Medium),
                        AquaticUdpRunner::new(8, 2, Priority::High),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(6, Priority::High),
                        OpenTrackerUdpRunner::new(12, Priority::Medium),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (6, Priority::Medium),
                    (8, Priority::Medium),
                    (12, Priority::High),
                    (16, Priority::High),
                    (24, Priority::Medium),
                ]),
            },
            8 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(7, 1, Priority::Medium),
                        AquaticUdpRunner::new(8, 1, Priority::Medium),
                        AquaticUdpRunner::new(14, 1, Priority::Low),
                        AquaticUdpRunner::new(6, 2, Priority::Low),
                        AquaticUdpRunner::new(12, 2, Priority::High),
                        AquaticUdpRunner::new(5, 3, Priority::Low),
                        AquaticUdpRunner::new(10, 3, Priority::Medium),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(8, Priority::High),
                        OpenTrackerUdpRunner::new(16, Priority::Medium),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::High),
                    (12, Priority::Medium),
                    (16, Priority::High),
                    (24, Priority::Medium)
                ]),
            },
            12 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(10, 2, Priority::Medium),
                        AquaticUdpRunner::new(12, 2, Priority::Medium),
                        AquaticUdpRunner::new(20, 2, Priority::Low),

                        AquaticUdpRunner::new(9, 3, Priority::Low),
                        AquaticUdpRunner::new(12, 3, Priority::Medium),
                        AquaticUdpRunner::new(18, 3, Priority::Low),

                        AquaticUdpRunner::new(8, 4, Priority::Low),
                        AquaticUdpRunner::new(12, 4, Priority::Medium),
                        AquaticUdpRunner::new(16, 4, Priority::High),

                        AquaticUdpRunner::new(7, 5, Priority::Low),
                        AquaticUdpRunner::new(12, 5, Priority::Medium),
                        AquaticUdpRunner::new(14, 5, Priority::Medium),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(12, Priority::High),
                        OpenTrackerUdpRunner::new(24, Priority::Medium),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::Medium),
                    (12, Priority::Medium),
                    (16, Priority::High),
                    (24, Priority::High),
                ]),
            },
            16 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::new(14, 2, Priority::Low),
                        AquaticUdpRunner::new(16, 2, Priority::Low),
                        AquaticUdpRunner::new(28, 2, Priority::Low),

                        AquaticUdpRunner::new(13, 3, Priority::Low),
                        AquaticUdpRunner::new(16, 3, Priority::Low),
                        AquaticUdpRunner::new(26, 3, Priority::Low),

                        AquaticUdpRunner::new(12, 4, Priority::Medium),
                        AquaticUdpRunner::new(16, 4, Priority::Medium),
                        AquaticUdpRunner::new(24, 4, Priority::Low),

                        AquaticUdpRunner::new(11, 5, Priority::Low),
                        AquaticUdpRunner::new(16, 5, Priority::Medium),
                        AquaticUdpRunner::new(22, 5, Priority::Low),

                        AquaticUdpRunner::new(10, 6, Priority::Low),
                        AquaticUdpRunner::new(16, 6, Priority::High),
                        AquaticUdpRunner::new(20, 6, Priority::Medium),

                        AquaticUdpRunner::new(9, 7, Priority::Low),
                        AquaticUdpRunner::new(16, 7, Priority::Medium),
                        AquaticUdpRunner::new(18, 7, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(16, Priority::High),
                        OpenTrackerUdpRunner::new(32, Priority::Medium),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::High),
                    (12, Priority::High),
                    (16, Priority::High),
                    (24, Priority::High),
                ]),
            },
        }
    }

    pub fn load_test_gen(
        parameters: LoadTestRunnerParameters,
    ) -> Box<dyn ProcessRunner<Command = UdpCommand>> {
        Box::new(AquaticUdpLoadTestRunner { parameters })
    }
}

#[derive(Debug, Clone)]
struct AquaticUdpRunner {
    socket_workers: usize,
    swarm_workers: usize,
    priority: Priority,
}

impl AquaticUdpRunner {
    #[allow(clippy::new_ret_no_self)]
    fn new(
        socket_workers: usize,
        swarm_workers: usize,
        priority: Priority,
    ) -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self {
            socket_workers,
            swarm_workers,
            priority,
        })
    }
}

impl ProcessRunner for AquaticUdpRunner {
    type Command = UdpCommand;

    #[allow(clippy::field_reassign_with_default)]
    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child> {
        let mut c = aquatic_udp::config::Config::default();

        c.socket_workers = self.socket_workers;
        c.swarm_workers = self.swarm_workers;
        c.network.address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 3000));
        c.protocol.max_response_peers = 30;

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

    fn priority(&self) -> crate::common::Priority {
        self.priority
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
    priority: Priority,
}

impl OpenTrackerUdpRunner {
    #[allow(clippy::new_ret_no_self)]
    fn new(workers: usize, priority: Priority) -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self { workers, priority })
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
            "listen.udp.workers {}\nlisten.udp 127.0.0.1:3000",
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

    fn priority(&self) -> crate::common::Priority {
        self.priority
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
    #[allow(clippy::new_ret_no_self)]
    fn new() -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self {})
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
                max_numwant: 30
                default_numwant: 30
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

    fn priority(&self) -> crate::common::Priority {
        Priority::High
    }

    fn keys(&self) -> IndexMap<String, String> {
        Default::default()
    }
}

#[derive(Debug, Clone)]
struct AquaticUdpLoadTestRunner {
    parameters: LoadTestRunnerParameters,
}

impl ProcessRunner for AquaticUdpLoadTestRunner {
    type Command = UdpCommand;

    #[allow(clippy::field_reassign_with_default)]
    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child> {
        let mut c = aquatic_udp_load_test::config::Config::default();

        c.workers = self.parameters.workers as u8;
        c.duration = self.parameters.duration;
        c.summarize_last = self.parameters.summarize_last;

        c.requests.announce_peers_wanted = 30;
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

    fn priority(&self) -> crate::common::Priority {
        eprintln!("load test runner priority method called");

        Priority::High
    }

    fn keys(&self) -> IndexMap<String, String> {
        indexmap! {
            "workers".to_string() => self.parameters.workers.to_string(),
        }
    }
}
