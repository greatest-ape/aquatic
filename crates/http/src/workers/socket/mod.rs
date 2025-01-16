mod connection;
mod request;

use std::cell::RefCell;
use std::net::SocketAddr;
use std::os::unix::prelude::{FromRawFd, IntoRawFd};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::access_list::AccessList;
use aquatic_common::privileges::PrivilegeDropper;
use aquatic_common::rustls_config::RustlsConfig;
use aquatic_common::{CanonicalSocketAddr, ServerStartInstant};
use arc_swap::{ArcSwap, ArcSwapAny};
use futures_lite::future::race;
use futures_lite::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::channels::local_channel::{new_bounded, LocalReceiver, LocalSender};
use glommio::net::{TcpListener, TcpStream};
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
    mut priv_droppers: Vec<PrivilegeDropper>,
    server_start_instant: ServerStartInstant,
    worker_index: usize,
) -> anyhow::Result<()> {
    let config = Rc::new(config);

    let tcp_listeners = {
        let opt_listener_ipv4 = if config.network.use_ipv4 {
            let priv_dropper = priv_droppers
                .pop()
                .ok_or(anyhow::anyhow!("no enough priv droppers"))?;
            let socket =
                create_tcp_listener(&config, priv_dropper, config.network.address_ipv4.into())
                    .context("create tcp listener")?;

            Some(socket)
        } else {
            None
        };
        let opt_listener_ipv6 = if config.network.use_ipv6 {
            let priv_dropper = priv_droppers
                .pop()
                .ok_or(anyhow::anyhow!("no enough priv droppers"))?;
            let socket =
                create_tcp_listener(&config, priv_dropper, config.network.address_ipv6.into())
                    .context("create tcp listener")?;

            Some(socket)
        } else {
            None
        };

        [opt_listener_ipv4, opt_listener_ipv6]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    };

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

    let tasks = tcp_listeners
        .into_iter()
        .map(|tcp_listener| {
            let listener_state = ListenerState {
                config: config.clone(),
                access_list: state.access_list.clone(),
                opt_tls_config: opt_tls_config.clone(),
                server_start_instant,
                connection_handles: connection_handles.clone(),
                request_senders: request_senders.clone(),
                worker_index,
            };

            spawn_local(listener_state.accept_connections(tcp_listener))
        })
        .collect::<Vec<_>>();

    for task in tasks {
        task.await;
    }

    Ok(())
}

#[derive(Clone)]
struct ListenerState {
    config: Rc<Config>,
    access_list: Arc<ArcSwapAny<Arc<AccessList>>>,
    opt_tls_config: Option<Arc<ArcSwap<RustlsConfig>>>,
    server_start_instant: ServerStartInstant,
    connection_handles: Rc<RefCell<HopSlotMap<ConnectionId, ConnectionHandle>>>,
    request_senders: Rc<Senders<ChannelRequest>>,
    worker_index: usize,
}

impl ListenerState {
    async fn accept_connections(self, listener: TcpListener) {
        let mut incoming = listener.incoming();

        while let Some(stream) = incoming.next().await {
            match stream {
                Ok(stream) => {
                    let (close_conn_sender, close_conn_receiver) = new_bounded(1);

                    let valid_until = Rc::new(RefCell::new(ValidUntil::new(
                        self.server_start_instant,
                        self.config.cleaning.max_connection_idle,
                    )));

                    let connection_id =
                        self.connection_handles
                            .borrow_mut()
                            .insert(ConnectionHandle {
                                close_conn_sender,
                                valid_until: valid_until.clone(),
                            });

                    spawn_local(self.clone().handle_connection(
                        close_conn_receiver,
                        valid_until,
                        connection_id,
                        stream,
                    ))
                    .detach();
                }
                Err(err) => {
                    ::log::error!("accept connection: {:?}", err);
                }
            }
        }
    }

    async fn handle_connection(
        self,
        close_conn_receiver: LocalReceiver<()>,
        valid_until: Rc<RefCell<ValidUntil>>,
        connection_id: ConnectionId,
        stream: TcpStream,
    ) {
        #[cfg(feature = "metrics")]
        let active_connections_gauge = ::metrics::gauge!(
            "aquatic_active_connections",
            "worker_index" => self.worker_index.to_string(),
        );

        #[cfg(feature = "metrics")]
        active_connections_gauge.increment(1.0);

        let f1 = async {
            run_connection(
                self.config,
                self.access_list,
                self.request_senders,
                self.server_start_instant,
                self.opt_tls_config,
                valid_until.clone(),
                stream,
                self.worker_index,
            )
            .await
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
            Err(
                err @ (ConnectionError::ResponseBufferWrite(_)
                | ConnectionError::ResponseBufferFull
                | ConnectionError::ScrapeChannelError(_)
                | ConnectionError::ResponseSenderClosed),
            ) => {
                ::log::error!("connection closed: {:#}", err);
            }
            Err(err @ ConnectionError::RequestBufferFull) => {
                ::log::info!("connection closed: {:#}", err);
            }
            Err(err) => {
                ::log::debug!("connection closed: {:#}", err);
            }
        }

        self.connection_handles.borrow_mut().remove(connection_id);
    }
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
    address: SocketAddr,
) -> anyhow::Result<TcpListener> {
    let socket = if address.is_ipv4() {
        socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?
    } else {
        let socket = socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;

        if config.network.set_only_ipv6 {
            socket
                .set_only_v6(true)
                .with_context(|| "socket: set only ipv6")?;
        }

        socket
    };

    socket
        .set_reuse_port(true)
        .with_context(|| "socket: set reuse port")?;

    socket
        .bind(&address.into())
        .with_context(|| format!("socket: bind to {}", address))?;

    socket
        .listen(config.network.tcp_backlog)
        .with_context(|| format!("socket: listen on {}", address))?;

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
