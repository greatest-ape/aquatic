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
    AquaticIoUring,
    OpenTracker,
    Chihaya,
    TorrustTracker,
}

impl Tracker for UdpTracker {
    fn name(&self) -> String {
        match self {
            Self::Aquatic => "aquatic_udp".into(),
            Self::AquaticIoUring => "aquatic_udp (io_uring)".into(),
            Self::OpenTracker => "opentracker".into(),
            Self::Chihaya => "chihaya".into(),
            Self::TorrustTracker => "torrust-tracker".into(),
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
    /// Path to torrust-tracker binary
    #[arg(long, default_value = "torrust-tracker")]
    torrust_tracker: PathBuf,
}

impl UdpCommand {
    pub fn sets(&self, cpu_mode: CpuMode) -> IndexMap<usize, SetConfig<UdpCommand, UdpTracker>> {
        // Priorities are based on what has previously produced the best results
        indexmap::indexmap! {
            1 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::with_mio(1, Priority::High),
                        // Allow running two workers per core for aquatic and
                        // opentracker. Skip this priority if testing on a
                        // virtual machine
                        AquaticUdpRunner::with_mio(2, Priority::Low),
                    ],
                    UdpTracker::AquaticIoUring => vec![
                        AquaticUdpRunner::with_io_uring(1, Priority::High),
                        AquaticUdpRunner::with_io_uring(2, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(0, Priority::Medium), // Handle requests within event loop
                        OpenTrackerUdpRunner::new(1, Priority::High),
                        OpenTrackerUdpRunner::new(2, Priority::Low),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                    UdpTracker::TorrustTracker => vec![
                        TorrustTrackerUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::Medium),
                    (12, Priority::High)
                ]),
            },
            2 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::with_mio(2, Priority::High),
                        AquaticUdpRunner::with_mio(4, Priority::Low),
                    ],
                    UdpTracker::AquaticIoUring => vec![
                        AquaticUdpRunner::with_io_uring(2, Priority::High),
                        AquaticUdpRunner::with_io_uring(4, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(2, Priority::High),
                        OpenTrackerUdpRunner::new(4, Priority::Low),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                    UdpTracker::TorrustTracker => vec![
                        TorrustTrackerUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::Medium),
                    (12, Priority::High),
                ]),
            },
            4 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::with_mio(4, Priority::High),
                        AquaticUdpRunner::with_mio(8, Priority::Low),
                    ],
                    UdpTracker::AquaticIoUring => vec![
                        AquaticUdpRunner::with_io_uring(4, Priority::High),
                        AquaticUdpRunner::with_io_uring(8, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(4, Priority::High),
                        OpenTrackerUdpRunner::new(8, Priority::Low),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                    UdpTracker::TorrustTracker => vec![
                        TorrustTrackerUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::Medium),
                    (12, Priority::High),
                ]),
            },
            6 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::with_mio(6, Priority::High),
                        AquaticUdpRunner::with_mio(12, Priority::Low),
                    ],
                    UdpTracker::AquaticIoUring => vec![
                        AquaticUdpRunner::with_io_uring(6, Priority::High),
                        AquaticUdpRunner::with_io_uring(12, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(6, Priority::High),
                        OpenTrackerUdpRunner::new(12, Priority::Low),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                    UdpTracker::TorrustTracker => vec![
                        TorrustTrackerUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::Medium),
                    (12, Priority::High),
                ]),
            },
            8 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::with_mio(8, Priority::High),
                        AquaticUdpRunner::with_mio(16, Priority::Low),
                    ],
                    UdpTracker::AquaticIoUring => vec![
                        AquaticUdpRunner::with_io_uring(8, Priority::High),
                        AquaticUdpRunner::with_io_uring(16, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(8, Priority::High),
                        OpenTrackerUdpRunner::new(16, Priority::Low),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                    UdpTracker::TorrustTracker => vec![
                        TorrustTrackerUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::Medium),
                    (12, Priority::High),
                ]),
            },
            12 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::with_mio(12, Priority::High),
                        AquaticUdpRunner::with_mio(24, Priority::Low),
                    ],
                    UdpTracker::AquaticIoUring => vec![
                        AquaticUdpRunner::with_io_uring(12, Priority::High),
                        AquaticUdpRunner::with_io_uring(24, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(12, Priority::High),
                        OpenTrackerUdpRunner::new(24, Priority::Low),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                    UdpTracker::TorrustTracker => vec![
                        TorrustTrackerUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::Medium),
                    (12, Priority::High),
                ]),
            },
            16 => SetConfig {
                implementations: indexmap! {
                    UdpTracker::Aquatic => vec![
                        AquaticUdpRunner::with_mio(16, Priority::High),
                        AquaticUdpRunner::with_mio(32, Priority::Low),
                    ],
                    UdpTracker::AquaticIoUring => vec![
                        AquaticUdpRunner::with_io_uring(16, Priority::High),
                        AquaticUdpRunner::with_io_uring(32, Priority::Low),
                    ],
                    UdpTracker::OpenTracker => vec![
                        OpenTrackerUdpRunner::new(16, Priority::High),
                        OpenTrackerUdpRunner::new(32, Priority::Low),
                    ],
                    UdpTracker::Chihaya => vec![
                        ChihayaUdpRunner::new(),
                    ],
                    UdpTracker::TorrustTracker => vec![
                        TorrustTrackerUdpRunner::new(),
                    ],
                },
                load_test_runs: simple_load_test_runs(cpu_mode, &[
                    (8, Priority::High),
                    (12, Priority::High),
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
    use_io_uring: bool,
    priority: Priority,
}

impl AquaticUdpRunner {
    fn with_mio(
        socket_workers: usize,
        priority: Priority,
    ) -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self {
            socket_workers,
            use_io_uring: false,
            priority,
        })
    }
    fn with_io_uring(
        socket_workers: usize,
        priority: Priority,
    ) -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self {
            socket_workers,
            use_io_uring: true,
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
        c.network.address_ipv4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 3000);
        c.network.use_ipv6 = false;
        c.network.use_io_uring = self.use_io_uring;
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
struct TorrustTrackerUdpRunner;

impl TorrustTrackerUdpRunner {
    #[allow(clippy::new_ret_no_self)]
    fn new() -> Rc<dyn ProcessRunner<Command = UdpCommand>> {
        Rc::new(Self {})
    }
}

impl ProcessRunner for TorrustTrackerUdpRunner {
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
            [metadata]
            schema_version = "2.0.0"

            [logging]
            threshold = "error"

            [core]
            listed = false
            private = false
            tracker_usage_statistics = false

            [core.database]
            driver = "sqlite3"
            path = "./sqlite3.db"

            [core.tracker_policy]
            persistent_torrent_completed_stat = false
            remove_peerless_torrents = false

            [[udp_trackers]]
            bind_address = "0.0.0.0:3000"
            "#,
        )?;

        Ok(Command::new("taskset")
            .arg("--cpu-list")
            .arg(vcpus.as_cpu_list())
            .env("TORRUST_TRACKER_CONFIG_TOML_PATH", tmp_file.path())
            .arg(&command.torrust_tracker)
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

        c.extra_statistics = false;

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
