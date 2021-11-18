use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpuPinningConfig {
    pub active: bool,
    pub mode: CpuPinningMode,
    pub virtual_per_physical_cpu: usize,
    pub offset_cpus: usize,
}

impl Default for CpuPinningConfig {
    fn default() -> Self {
        Self {
            active: false,
            mode: Default::default(),
            virtual_per_physical_cpu: 2,
            offset_cpus: 0,
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
    Other,
}

impl WorkerIndex {
    fn get_cpu_indices(self, config: &CpuPinningConfig, socket_workers: usize) -> Vec<usize> {
        let offset = match self {
            Self::Other => config.virtual_per_physical_cpu * config.offset_cpus,
            Self::SocketWorker(index) => {
                config.virtual_per_physical_cpu * (config.offset_cpus + 1 + index)
            }
            Self::RequestWorker(index) => {
                config.virtual_per_physical_cpu * (config.offset_cpus + 1 + socket_workers + index)
            }
        };

        let virtual_cpus = (0..config.virtual_per_physical_cpu).map(|i| offset + i);

        let virtual_cpus: Vec<usize> = match config.mode {
            CpuPinningMode::Ascending => virtual_cpus.collect(),
            CpuPinningMode::Descending => {
                let max_index = affinity::get_core_num() - 1;

                virtual_cpus
                    .map(|i| max_index.checked_sub(i).unwrap_or(0))
                    .collect()
            }
        };

        ::log::info!(
            "Calculated virtual CPU pin indices {:?} for {:?}",
            virtual_cpus,
            self
        );

        virtual_cpus
    }
}

/// Note: don't call this when affinities were already set in the current or in
/// a parent thread. Doing so limits the number of cores that are seen and
/// messes up setting affinities.
pub fn pin_current_if_configured_to(
    config: &CpuPinningConfig,
    socket_workers: usize,
    worker_index: WorkerIndex,
) {
    if config.active {
        let indices = worker_index.get_cpu_indices(config, socket_workers);

        if let Err(err) = affinity::set_thread_affinity(indices.clone()) {
            ::log::error!(
                "Failed setting thread affinities {:?} for {:?}: {:#?}",
                indices,
                worker_index,
                err
            );
        }
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
