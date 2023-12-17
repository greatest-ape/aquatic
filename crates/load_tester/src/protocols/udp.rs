use std::{
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    rc::Rc,
};

use clap::Parser;
use indexmap::{indexmap, IndexMap};
use tempfile::NamedTempFile;

use crate::{
    common::{simple_load_test_runs, CpuMode, TaskSetCpuList},
    run::ProcessRunner,
    set::{run_sets, Server, SetConfig},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UdpServer {
    Aquatic,
    OpenTracker,
}

impl Server for UdpServer {
    fn name(&self) -> String {
        match self {
            Self::Aquatic => "aquatic_udp".into(),
            Self::OpenTracker => "opentracker".into(),
        }
    }
}

#[derive(Parser, Debug)]
pub struct UdpCommand {
    #[arg(long, default_value_t = CpuMode::Split)]
    cpu_mode: CpuMode,
    #[arg(long, default_value = "./target/release-debug/aquatic_udp_load_test")]
    load_test: PathBuf,
    #[arg(long, default_value = "./target/release-debug/aquatic_udp")]
    aquatic: PathBuf,
    #[arg(long, default_value = "opentracker")]
    opentracker: PathBuf,
}

impl UdpCommand {
    pub fn run(&self) -> anyhow::Result<()> {
        run_sets(self, self.cpu_mode, self.sets(), |workers| {
            Box::new(AquaticUdpLoadTestProcessConfig { workers })
        });

        Ok(())
    }

    fn sets(&self) -> IndexMap<usize, SetConfig<UdpCommand, UdpServer>> {
        indexmap::indexmap! {
            1 => SetConfig {
                implementations: indexmap! {
                    UdpServer::Aquatic => vec![
                        Rc::new(AquaticUdpProcessConfig {
                            socket_workers: 1,
                            swarm_workers: 1,
                        }) as Rc<dyn ProcessRunner<Command = UdpCommand>>,
                    ],
                    /*
                    UdpServer::OpenTracker => vec![
                        Rc::new(OpenTrackerUdpProcessConfig {
                            workers: 1,
                        }) as Rc<dyn RunProcess<Command = UdpCommand>>,
                        Rc::new(OpenTrackerUdpProcessConfig {
                            workers: 2,
                        }) as Rc<dyn RunProcess<Command = UdpCommand>>,
                    ],
                    */
                },
                load_test_runs: simple_load_test_runs(self.cpu_mode, &[1, 2, 4]),
            },
            2 => SetConfig {
                implementations: indexmap! {
                    UdpServer::Aquatic => vec![
                        Rc::new(AquaticUdpProcessConfig {
                            socket_workers: 1,
                            swarm_workers: 1,
                        }) as Rc<dyn ProcessRunner<Command = UdpCommand>>,
                        Rc::new(AquaticUdpProcessConfig {
                            socket_workers: 2,
                            swarm_workers: 1,
                        }) as Rc<dyn ProcessRunner<Command = UdpCommand>>,
                    ],
                    /*
                    UdpServer::OpenTracker => vec![
                        Rc::new(OpenTrackerUdpProcessConfig {
                            workers: 2,
                        }) as Rc<dyn RunProcess<Command = UdpCommand>>,
                        Rc::new(OpenTrackerUdpProcessConfig {
                            workers: 4,
                        }) as Rc<dyn RunProcess<Command = UdpCommand>>,
                    ],
                    */
                },
                load_test_runs: simple_load_test_runs(self.cpu_mode, &[1, 2, 4]),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct AquaticUdpProcessConfig {
    socket_workers: usize,
    swarm_workers: usize,
}

impl ProcessRunner for AquaticUdpProcessConfig {
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

    fn info(&self) -> String {
        format!(
            "socket workers: {}, swarm workers: {}",
            self.socket_workers, self.swarm_workers
        )
    }
    fn keys(&self) -> IndexMap<String, String> {
        indexmap! {
            "socket workers".to_string() => self.socket_workers.to_string(),
            "swarm workers".to_string() => self.swarm_workers.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenTrackerUdpProcessConfig {
    workers: usize,
}

impl ProcessRunner for OpenTrackerUdpProcessConfig {
    type Command = UdpCommand;

    fn run(
        &self,
        command: &Self::Command,
        vcpus: &TaskSetCpuList,
        tmp_file: &mut NamedTempFile,
    ) -> anyhow::Result<Child> {
        writeln!(tmp_file, "{}", self.workers)?; // FIXME

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

    fn info(&self) -> String {
        format!("workers: {}", self.workers)
    }

    fn keys(&self) -> IndexMap<String, String> {
        indexmap! {
            "workers".to_string() => self.workers.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AquaticUdpLoadTestProcessConfig {
    workers: usize,
}

impl ProcessRunner for AquaticUdpLoadTestProcessConfig {
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

    fn info(&self) -> String {
        format!("workers: {}", self.workers)
    }

    fn keys(&self) -> IndexMap<String, String> {
        indexmap! {
            "workers".to_string() => self.workers.to_string(),
        }
    }
}
