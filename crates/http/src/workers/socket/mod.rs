mod connection;
mod request;

use std::cell::RefCell;
use std::os::unix::prelude::{FromRawFd, IntoRawFd};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::privileges::PrivilegeDropper;
use aquatic_common::rustls_config::RustlsConfig;
use aquatic_common::{CanonicalSocketAddr, ServerStartInstant};
use arc_swap::ArcSwap;
use futures_lite::future::race;
use futures_lite::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role};
use glommio::channels::local_channel::{new_bounded, LocalSender};
use glommio::net::TcpListener;
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use slotmap::HopSlotMap;

use crate::common::*;
use crate::config::Config;
use crate::workers::socket::connection::{run_connection, ConnectionError};

struct ConnectionHandle {
    close_conn_sender: LocalSender<()>,
    valid_until: Rc<RefCell<ValidUntil>>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_socket_worker(
    config: Config,
    state: State,
    opt_tls_config: Option<Arc<ArcSwap<RustlsConfig>>>,
    request_mesh_builder: MeshBuilder<ChannelRequest, Partial>,
    priv_dropper: PrivilegeDropper,
    server_start_instant: ServerStartInstant,
    worker_index: usize,
) -> anyhow::Result<()> {
    let config = Rc::new(config);
    let access_list = state.access_list;

    let listener = create_tcp_listener(&config, priv_dropper).context("create tcp listener")?;

    let (request_senders, _) = request_mesh_builder
        .join(Role::Producer)
        .await
        .map_err(|err| anyhow::anyhow!("join request mesh: {:#}", err))?;
    let request_senders = Rc::new(request_senders);

    let connection_handles = Rc::new(RefCell::new(HopSlotMap::with_key()));

    TimerActionRepeat::repeat(enclose!((config, connection_handles) move || {
        clean_connections(
            config.clone(),
            connection_handles.clone(),
            server_start_instant,
        )
    }));

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let (close_conn_sender, close_conn_receiver) = new_bounded(1);

                let valid_until = Rc::new(RefCell::new(ValidUntil::new(
                    server_start_instant,
                    config.cleaning.max_connection_idle,
                )));

                let connection_id = connection_handles.borrow_mut().insert(ConnectionHandle {
                    close_conn_sender,
                    valid_until: valid_until.clone(),
                });

                spawn_local(enclose!(
                    (
                        config,
                        access_list,
                        request_senders,
                        opt_tls_config,
                        connection_handles,
                        valid_until,
                    )
                    async move {
                        #[cfg(feature = "metrics")]
                        let active_connections_gauge = ::metrics::gauge!(
                            "aquatic_active_connections",
                            "worker_index" => worker_index.to_string(),
                        );

                        #[cfg(feature = "metrics")]
                        active_connections_gauge.increment(1.0);

                        let f1 = async { run_connection(
                                config,
                                access_list,
                                request_senders,
                                server_start_instant,
                                opt_tls_config,
                                valid_until.clone(),
                                stream,
                                worker_index,
                            ).await
                        };
                        let f2 = async {
                            close_conn_receiver.recv().await;

                            Err(ConnectionError::Inactive)
                        };

                        let result = race(f1, f2).await;

                        #[cfg(feature = "metrics")]
                        active_connections_gauge.decrement(1.0);

                        match result {
                            Ok(()) => (),
                            Err(err@(
                                ConnectionError::ResponseBufferWrite(_) |
                                ConnectionError::ResponseBufferFull |
                                ConnectionError::ScrapeChannelError(_) |
                                ConnectionError::ResponseSenderClosed
                            )) => {
                                ::log::error!("connection closed: {:#}", err);
                            }
                            Err(err@ConnectionError::RequestBufferFull) => {
                                ::log::info!("connection closed: {:#}", err);
                            }
                            Err(err) => {
                                ::log::debug!("connection closed: {:#}", err);
                            }
                        }

                        connection_handles.borrow_mut().remove(connection_id);
                    }
                ))
                .detach();
            }
            Err(err) => {
                ::log::error!("accept connection: {:?}", err);
            }
        }
    }

    Ok(())
}

async fn clean_connections(
    config: Rc<Config>,
    connection_slab: Rc<RefCell<HopSlotMap<ConnectionId, ConnectionHandle>>>,
    server_start_instant: ServerStartInstant,
) -> Option<Duration> {
    let now = server_start_instant.seconds_elapsed();

    connection_slab.borrow_mut().retain(|_, handle| {
        if handle.valid_until.borrow().valid(now) {
            true
        } else {
            let _ = handle.close_conn_sender.try_send(());

            false
        }
    });

    Some(Duration::from_secs(
        config.cleaning.connection_cleaning_interval,
    ))
}

fn create_tcp_listener(
    config: &Config,
    priv_dropper: PrivilegeDropper,
) -> anyhow::Result<TcpListener> {
    let domain = if config.network.address.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;

    if config.network.only_ipv6 {
        socket
            .set_only_v6(true)
            .with_context(|| "socket: set only ipv6")?;
    }

    socket
        .set_reuse_port(true)
        .with_context(|| "socket: set reuse port")?;

    socket
        .bind(&config.network.address.into())
        .with_context(|| format!("socket: bind to {}", config.network.address))?;

    socket
        .listen(config.network.tcp_backlog)
        .with_context(|| format!("socket: listen on {}", config.network.address))?;

    priv_dropper.after_socket_creation()?;

    Ok(unsafe { TcpListener::from_raw_fd(socket.into_raw_fd()) })
}

#[cfg(feature = "metrics")]
fn peer_addr_to_ip_version_str(addr: &CanonicalSocketAddr) -> &'static str {
    if addr.is_ipv4() {
        "4"
    } else {
        "6"
    }
}
