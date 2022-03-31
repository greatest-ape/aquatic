use aquatic_toml_config::TomlConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, TomlConfig, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CpuPinningMode {
    Ascending,
    Descending,
}

impl Default for CpuPinningMode {
    fn default() -> Self {
        Self::Ascending
    }
}

#[cfg(feature = "with-glommio")]
#[derive(Clone, Debug, PartialEq, TomlConfig, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HyperThreadMapping {
    System,
    Subsequent,
    Split,
}

#[cfg(feature = "with-glommio")]
impl Default for HyperThreadMapping {
    fn default() -> Self {
        Self::System
    }
}

/// Experimental CPU pinning
#[derive(Clone, Debug, PartialEq, TomlConfig, Deserialize)]
pub struct CpuPinningConfig {
    pub active: bool,
    pub mode: CpuPinningMode,
    #[cfg(feature = "with-glommio")]
    pub hyperthread: HyperThreadMapping,
    pub core_offset: usize,
}

impl Default for CpuPinningConfig {
    fn default() -> Self {
        Self {
            active: false,
            mode: Default::default(),
            #[cfg(feature = "with-glommio")]
            hyperthread: Default::default(),
            core_offset: 0,
        }
    }
}

impl CpuPinningConfig {
    pub fn default_for_load_test() -> Self {
        Self {
            mode: CpuPinningMode::Descending,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum WorkerIndex {
    SocketWorker(usize),
    RequestWorker(usize),
    Util,
}

impl WorkerIndex {
    pub fn get_core_index(
        &self,
        config: &CpuPinningConfig,
        socket_workers: usize,
        request_workers: usize,
        num_cores: usize,
    ) -> usize {
        let ascending_index = match self {
            Self::SocketWorker(index) => config.core_offset + index,
            Self::RequestWorker(index) => config.core_offset + socket_workers + index,
            Self::Util => config.core_offset + socket_workers + request_workers,
        };

        let max_core_index = num_cores - 1;

        let ascending_index = ascending_index.min(max_core_index);

        match config.mode {
            CpuPinningMode::Ascending => ascending_index,
            CpuPinningMode::Descending => max_core_index - ascending_index,
        }
    }
}

#[cfg(feature = "with-glommio")]
pub mod glommio {
    use ::glommio::{CpuSet, Placement};

    use super::*;

    fn get_cpu_set() -> anyhow::Result<CpuSet> {
        CpuSet::online().map_err(|err| anyhow::anyhow!("Couldn't get CPU set: {:#}", err))
    }

    fn get_num_cpu_cores() -> anyhow::Result<usize> {
        get_cpu_set()?
            .iter()
            .map(|l| l.core)
            .max()
            .map(|index| index + 1)
            .ok_or(anyhow::anyhow!("CpuSet is empty"))
    }

    fn logical_cpus_string(cpu_set: &CpuSet) -> String {
        let mut logical_cpus = cpu_set.iter().map(|l| l.cpu).collect::<Vec<usize>>();

        logical_cpus.sort_unstable();

        logical_cpus
            .into_iter()
            .map(|cpu| cpu.to_string())
            .collect::<Vec<String>>()
            .join(", ")
    }

    fn get_worker_cpu_set(
        config: &CpuPinningConfig,
        socket_workers: usize,
        request_workers: usize,
        worker_index: WorkerIndex,
    ) -> anyhow::Result<CpuSet> {
        let num_cpu_cores = get_num_cpu_cores()?;

        let core_index =
            worker_index.get_core_index(&config, socket_workers, request_workers, num_cpu_cores);

        let too_many_workers = match (&config.hyperthread, &config.mode) {
            (
                HyperThreadMapping::Split | HyperThreadMapping::Subsequent,
                CpuPinningMode::Ascending,
            ) => core_index >= num_cpu_cores / 2,
            (
                HyperThreadMapping::Split | HyperThreadMapping::Subsequent,
                CpuPinningMode::Descending,
            ) => core_index < num_cpu_cores / 2,
            (_, _) => false,
        };

        if too_many_workers {
            return Err(anyhow::anyhow!("CPU pinning: total number of workers (including the single utility worker) can not exceed number of virtual CPUs / 2 - core_offset in this hyperthread mapping mode"));
        }

        let cpu_set = match config.hyperthread {
            HyperThreadMapping::System => get_cpu_set()?.filter(|l| l.core == core_index),
            HyperThreadMapping::Split => match config.mode {
                CpuPinningMode::Ascending => get_cpu_set()?
                    .filter(|l| l.cpu == core_index || l.cpu == core_index + num_cpu_cores / 2),
                CpuPinningMode::Descending => get_cpu_set()?
                    .filter(|l| l.cpu == core_index || l.cpu == core_index - num_cpu_cores / 2),
            },
            HyperThreadMapping::Subsequent => {
                let cpu_index_offset = match config.mode {
                    // 0 -> 0 and 1
                    // 1 -> 2 and 3
                    // 2 -> 4 and 5
                    CpuPinningMode::Ascending => core_index * 2,
                    // 15 -> 14 and 15
                    // 14 -> 12 and 13
                    // 13 -> 10 and 11
                    CpuPinningMode::Descending => num_cpu_cores - 2 * (num_cpu_cores - core_index),
                };

                get_cpu_set()?
                    .filter(|l| l.cpu == cpu_index_offset || l.cpu == cpu_index_offset + 1)
            }
        };

        if cpu_set.is_empty() {
            Err(anyhow::anyhow!(
                "CPU pinning: produced empty CPU set for {:?}. Try decreasing number of workers",
                worker_index
            ))
        } else {
            ::log::info!(
                "Logical CPUs for {:?}: {}",
                worker_index,
                logical_cpus_string(&cpu_set)
            );

            Ok(cpu_set)
        }
    }

    pub fn get_worker_placement(
        config: &CpuPinningConfig,
        socket_workers: usize,
        request_workers: usize,
        worker_index: WorkerIndex,
    ) -> anyhow::Result<Placement> {
        if config.active {
            let cpu_set =
                get_worker_cpu_set(&config, socket_workers, request_workers, worker_index)?;

            Ok(Placement::Fenced(cpu_set))
        } else {
            Ok(Placement::Unbound)
        }
    }

    pub fn set_affinity_for_util_worker(
        config: &CpuPinningConfig,
        socket_workers: usize,
        request_workers: usize,
    ) -> anyhow::Result<()> {
        let worker_cpu_set =
            get_worker_cpu_set(&config, socket_workers, request_workers, WorkerIndex::Util)?;

        unsafe {
            let mut set: libc::cpu_set_t = ::std::mem::zeroed();

            for cpu_location in worker_cpu_set {
                libc::CPU_SET(cpu_location.cpu, &mut set);
            }

            let status = libc::pthread_setaffinity_np(
                libc::pthread_self(),
                ::std::mem::size_of::<libc::cpu_set_t>(),
                &set,
            );

            if status != 0 {
                return Err(anyhow::Error::new(::std::io::Error::from_raw_os_error(
                    status,
                )));
            }
        }

        Ok(())
    }
}

/// Pin current thread to a suitable core
///
/// Requires hwloc (`apt-get install libhwloc-dev`)
#[cfg(feature = "with-hwloc")]
pub fn pin_current_if_configured_to(
    config: &CpuPinningConfig,
    socket_workers: usize,
    request_workers: usize,
    worker_index: WorkerIndex,
) {
    use hwloc::{CpuSet, ObjectType, Topology, CPUBIND_THREAD};

    if config.active {
        let mut topology = Topology::new();

        let core_cpu_sets: Vec<CpuSet> = topology
            .objects_with_type(&ObjectType::Core)
            .expect("hwloc: list cores")
            .into_iter()
            .map(|core| core.allowed_cpuset().expect("hwloc: get core cpu set"))
            .collect();

        let num_cores = core_cpu_sets.len();

        let core_index =
            worker_index.get_core_index(config, socket_workers, request_workers, num_cores);

        let cpu_set = core_cpu_sets
            .get(core_index)
            .expect(&format!("get cpu set for core {}", core_index))
            .to_owned();

        topology
            .set_cpubind(cpu_set, CPUBIND_THREAD)
            .expect(&format!("bind thread to core {}", core_index));

        ::log::info!(
            "Pinned worker {:?} to cpu core {}",
            worker_index,
            core_index
        );
    }
}

/// Tell Linux that incoming messages should be handled by the socket worker
/// with the same index as the CPU core receiving the interrupt.
///
/// Requires that sockets are actually bound in order, so waiting has to be done
/// in socket workers.
///
/// It might make sense to first enable RSS or RPS (if hardware doesn't support
/// RSS) and enable sending interrupts to all CPUs that have socket workers
/// running on them. Possibly, CPU 0 should be excluded.
///
/// More Information:
///   - https://talawah.io/blog/extreme-http-performance-tuning-one-point-two-million/
///   - https://www.kernel.org/doc/Documentation/networking/scaling.txt
///   - https://access.redhat.com/documentation/en-us/red_hat_enterprise_linux/6/html/performance_tuning_guide/network-rps
#[cfg(target_os = "linux")]
pub fn socket_attach_cbpf<S: ::std::os::unix::prelude::AsRawFd>(
    socket: &S,
    _num_sockets: usize,
) -> ::std::io::Result<()> {
    use std::mem::size_of;
    use std::os::raw::c_void;

    use libc::{setsockopt, sock_filter, sock_fprog, SOL_SOCKET, SO_ATTACH_REUSEPORT_CBPF};

    // Good BPF documentation: https://man.openbsd.org/bpf.4

    // Values of constants were copied from the following Linux source files:
    //   - include/uapi/linux/bpf_common.h
    //   - include/uapi/linux/filter.h

    // Instruction
    const BPF_LD: u16 = 0x00; // Load into A
                              // const BPF_LDX: u16 = 0x01; // Load into X
                              // const BPF_ALU: u16 = 0x04; // Load into X
    const BPF_RET: u16 = 0x06; // Return value
                               // const BPF_MOD: u16 = 0x90; // Run modulo on A

    // Size
    const BPF_W: u16 = 0x00; // 32-bit width

    // Source
    // const BPF_IMM: u16 = 0x00; // Use constant (k)
    const BPF_ABS: u16 = 0x20;

    // Registers
    // const BPF_K: u16 = 0x00;
    const BPF_A: u16 = 0x10;

    // k
    const SKF_AD_OFF: i32 = -0x1000; // Activate extensions
    const SKF_AD_CPU: i32 = 36; // Extension for getting CPU

    // Return index of socket that should receive packet
    let mut filter = [
        // Store index of CPU receiving packet in register A
        sock_filter {
            code: BPF_LD | BPF_W | BPF_ABS,
            jt: 0,
            jf: 0,
            k: u32::from_ne_bytes((SKF_AD_OFF + SKF_AD_CPU).to_ne_bytes()),
        },
        /* Disabled, because it doesn't make a lot of sense
        // Run A = A % socket_workers
        sock_filter {
            code: BPF_ALU | BPF_MOD,
            jt: 0,
            jf: 0,
            k: num_sockets as u32,
        },
        */
        // Return A
        sock_filter {
            code: BPF_RET | BPF_A,
            jt: 0,
            jf: 0,
            k: 0,
        },
    ];

    let program = sock_fprog {
        filter: filter.as_mut_ptr(),
        len: filter.len() as u16,
    };

    let program_ptr: *const sock_fprog = &program;

    unsafe {
        let result = setsockopt(
            socket.as_raw_fd(),
            SOL_SOCKET,
            SO_ATTACH_REUSEPORT_CBPF,
            program_ptr as *const c_void,
            size_of::<sock_fprog>() as u32,
        );

        if result != 0 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
