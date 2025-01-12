use std::fmt::Display;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::time::Instant;

use ahash::RandomState;

pub mod access_list;
pub mod cli;
#[cfg(feature = "cpu-pinning")]
pub mod cpu_pinning;
pub mod privileges;
#[cfg(feature = "rustls")]
pub mod rustls_config;

/// IndexMap using AHash hasher
pub type IndexMap<K, V> = indexmap::IndexMap<K, V, RandomState>;

/// Peer, connection or similar valid until this instant
#[derive(Debug, Clone, Copy)]
pub struct ValidUntil(SecondsSinceServerStart);

impl ValidUntil {
    #[inline]
    pub fn new(start_instant: ServerStartInstant, offset_seconds: u32) -> Self {
        Self(SecondsSinceServerStart(
            start_instant.seconds_elapsed().0 + offset_seconds,
        ))
    }
    pub fn new_with_now(now: SecondsSinceServerStart, offset_seconds: u32) -> Self {
        Self(SecondsSinceServerStart(now.0 + offset_seconds))
    }
    pub fn valid(&self, now: SecondsSinceServerStart) -> bool {
        self.0 .0 > now.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ServerStartInstant(Instant);

impl ServerStartInstant {
    #[allow(clippy::new_without_default)] // I prefer ::new here
    pub fn new() -> Self {
        Self(Instant::now())
    }
    pub fn seconds_elapsed(&self) -> SecondsSinceServerStart {
        SecondsSinceServerStart(
            self.0
                .elapsed()
                .as_secs()
                .try_into()
                .expect("server ran for more seconds than what fits in a u32"),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SecondsSinceServerStart(u32);

/// SocketAddr that is not an IPv6-mapped IPv4 address
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct CanonicalSocketAddr(SocketAddr);

impl CanonicalSocketAddr {
    pub fn new(addr: SocketAddr) -> Self {
        match addr {
            addr @ SocketAddr::V4(_) => Self(addr),
            SocketAddr::V6(addr) => {
                match addr.ip().octets() {
                    // Convert IPv4-mapped address (available in std but nightly-only)
                    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => Self(SocketAddr::V4(
                        SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), addr.port()),
                    )),
                    _ => Self(addr.into()),
                }
            }
        }
    }

    pub fn get_ipv6_mapped(self) -> SocketAddr {
        match self.0 {
            SocketAddr::V4(addr) => {
                let ip = addr.ip().to_ipv6_mapped();

                SocketAddr::V6(SocketAddrV6::new(ip, addr.port(), 0, 0))
            }
            addr => addr,
        }
    }

    pub fn get(self) -> SocketAddr {
        self.0
    }

    pub fn get_ipv4(self) -> Option<SocketAddr> {
        match self.0 {
            addr @ SocketAddr::V4(_) => Some(addr),
            _ => None,
        }
    }

    pub fn is_ipv4(&self) -> bool {
        self.0.is_ipv4()
    }
}

#[cfg(feature = "prometheus")]
pub fn spawn_prometheus_endpoint(
    addr: SocketAddr,
    timeout: Option<::std::time::Duration>,
    timeout_mask: Option<metrics_util::MetricKindMask>,
) -> anyhow::Result<::std::thread::JoinHandle<anyhow::Result<()>>> {
    use std::thread::Builder;
    use std::time::Duration;

    use anyhow::Context;

    let handle = Builder::new()
        .name("prometheus".into())
        .spawn(move || {
            use metrics_exporter_prometheus::PrometheusBuilder;
            use metrics_util::MetricKindMask;

            let rt = ::tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build prometheus tokio runtime")?;

            rt.block_on(async {
                let mask = timeout_mask.unwrap_or(MetricKindMask::ALL);

                let (recorder, exporter) = PrometheusBuilder::new()
                    .idle_timeout(mask, timeout)
                    .with_http_listener(addr)
                    .build()
                    .context("build prometheus recorder and exporter")?;

                let recorder_handle = recorder.handle();

                ::metrics::set_global_recorder(recorder).context("set global metrics recorder")?;

                ::tokio::spawn(async move {
                    let mut interval = ::tokio::time::interval(Duration::from_secs(5));

                    loop {
                        interval.tick().await;

                        // Periodically render metrics to make sure
                        // idles are cleaned up
                        recorder_handle.render();
                    }
                });

                exporter
                    .await
                    .map_err(|err| anyhow::anyhow!("run prometheus exporter: :{:#?}", err))
            })
        })
        .context("spawn prometheus endpoint")?;

    Ok(handle)
}

pub enum WorkerType {
    Swarm(usize),
    Socket(usize),
    Statistics,
    Signals,
    Cleaning,
    #[cfg(feature = "prometheus")]
    Prometheus,
}

impl Display for WorkerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Swarm(index) => f.write_fmt(format_args!("Swarm worker {}", index + 1)),
            Self::Socket(index) => f.write_fmt(format_args!("Socket worker {}", index + 1)),
            Self::Statistics => f.write_str("Statistics worker"),
            Self::Signals => f.write_str("Signals worker"),
            Self::Cleaning => f.write_str("Cleaning worker"),
            #[cfg(feature = "prometheus")]
            Self::Prometheus => f.write_str("Prometheus worker"),
        }
    }
}
