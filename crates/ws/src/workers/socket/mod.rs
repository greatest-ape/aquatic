use std::cell::RefCell;
use std::os::unix::prelude::{FromRawFd, IntoRawFd};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use aquatic_common::privileges::PrivilegeDropper;
use aquatic_common::rustls_config::RustlsConfig;
use aquatic_common::ServerStartInstant;
use aquatic_ws_protocol::common::InfoHash;
use aquatic_ws_protocol::incoming::InMessage;
use aquatic_ws_protocol::outgoing::OutMessage;
use arc_swap::ArcSwap;
use futures::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role};
use glommio::channels::local_channel::{new_bounded, LocalSender};
use glommio::channels::shared_channel::ConnectedReceiver;
use glommio::net::TcpListener;
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use slotmap::HopSlotMap;

use crate::config::Config;

use crate::common::*;
use crate::workers::socket::connection::ConnectionRunner;

mod connection;

type ConnectionHandles = HopSlotMap<ConnectionId, ConnectionHandle>;

const LOCAL_CHANNEL_SIZE: usize = 16;

#[cfg(feature = "metrics")]
thread_local! { static WORKER_INDEX: ::std::cell::Cell<usize> = Default::default() }

/// Used to interact with the connection tasks
struct ConnectionHandle {
    close_conn_sender: LocalSender<()>,
    /// Sender part of channel used to pass on outgoing messages from request
    /// worker
    out_message_sender: Rc<LocalSender<(OutMessageMeta, OutMessage)>>,
    /// Updated after sending message to peer
    valid_until: Rc<RefCell<ValidUntil>>,
    /// The TLS config used for this connection
    opt_tls_config: Option<Arc<RustlsConfig>>,
    valid_until_after_tls_update: Option<ValidUntil>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_socket_worker(
    config: Config,
    state: State,
    opt_tls_config: Option<Arc<ArcSwap<RustlsConfig>>>,
    control_message_mesh_builder: MeshBuilder<SwarmControlMessage, Partial>,
    in_message_mesh_builder: MeshBuilder<(InMessageMeta, InMessage), Partial>,
    out_message_mesh_builder: MeshBuilder<(OutMessageMeta, OutMessage), Partial>,
    priv_dropper: PrivilegeDropper,
    server_start_instant: ServerStartInstant,
    worker_index: usize,
) -> anyhow::Result<()> {
    #[cfg(feature = "metrics")]
    WORKER_INDEX.with(|index| index.set(worker_index));

    let config = Rc::new(config);
    let access_list = state.access_list;

    let listener = create_tcp_listener(&config, priv_dropper).context("create tcp listener")?;

    ::log::info!("created tcp listener");

    let (control_message_senders, _) = control_message_mesh_builder
        .join(Role::Producer)
        .await
        .map_err(|err| anyhow::anyhow!("join control message mesh: {:#}", err))?;
    let (in_message_senders, _) = in_message_mesh_builder
        .join(Role::Producer)
        .await
        .map_err(|err| anyhow::anyhow!("join in message mesh: {:#}", err))?;
    let (_, mut out_message_receivers) = out_message_mesh_builder
        .join(Role::Consumer)
        .await
        .map_err(|err| anyhow::anyhow!("join out message mesh: {:#}", err))?;

    let control_message_senders = Rc::new(control_message_senders);
    let in_message_senders = Rc::new(in_message_senders);

    let out_message_consumer_id = ConsumerId(
        out_message_receivers
            .consumer_id()
            .unwrap()
            .try_into()
            .unwrap(),
    );

    let tq_prioritized = executor().create_task_queue(
        Shares::Static(100),
        Latency::Matters(Duration::from_millis(1)),
        "prioritized",
    );
    let tq_regular =
        executor().create_task_queue(Shares::Static(1), Latency::NotImportant, "regular");

    ::log::info!("joined channels");

    let connection_handles = Rc::new(RefCell::new(ConnectionHandles::default()));

    // Periodically clean connections
    TimerActionRepeat::repeat_into(
        enclose!((config, connection_handles, opt_tls_config) move || {
            clean_connections(
                config.clone(),
                connection_handles.clone(),
                server_start_instant,
                opt_tls_config.clone(),
            )
        }),
        tq_prioritized,
    )
    .map_err(|err| anyhow::anyhow!("spawn connection cleaning task: {:#}", err))?;

    for (_, out_message_receiver) in out_message_receivers.streams() {
        spawn_local_into(
            receive_out_messages(out_message_receiver, connection_handles.clone()),
            tq_regular,
        )
        .map_err(|err| anyhow::anyhow!("spawn out message receiving task: {:#}", err))?
        .detach();
    }

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Err(err) => {
                ::log::error!("accept connection: {:#}", err);
            }
            Ok(stream) => {
                let ip_version = match stream.peer_addr() {
                    Ok(addr) => IpVersion::canonical_from_ip(addr.ip()),
                    Err(err) => {
                        ::log::info!("could not extract ip version (v4 or v6): {:#}", err);

                        continue;
                    }
                };

                let (out_message_sender, out_message_receiver) = new_bounded(LOCAL_CHANNEL_SIZE);
                let out_message_sender = Rc::new(out_message_sender);

                let (close_conn_sender, close_conn_receiver) = new_bounded(1);

                let connection_valid_until = Rc::new(RefCell::new(ValidUntil::new(
                    server_start_instant,
                    config.cleaning.max_connection_idle,
                )));

                let connection_handle = ConnectionHandle {
                    close_conn_sender,
                    out_message_sender: out_message_sender.clone(),
                    valid_until: connection_valid_until.clone(),
                    opt_tls_config: opt_tls_config.as_ref().map(|c| c.load_full()),
                    valid_until_after_tls_update: None,
                };

                let connection_id = connection_handles.borrow_mut().insert(connection_handle);

                spawn_local_into(
                    enclose!((
                        config,
                        access_list,
                        in_message_senders,
                        connection_valid_until,
                        opt_tls_config,
                        control_message_senders,
                        connection_handles
                    ) async move {
                        let runner = ConnectionRunner {
                            config,
                            access_list,
                            in_message_senders,
                            connection_valid_until,
                            out_message_sender,
                            out_message_receiver,
                            server_start_instant,
                            out_message_consumer_id,
                            connection_id,
                            opt_tls_config,
                            ip_version
                        };

                        runner.run(control_message_senders, close_conn_receiver, stream).await;

                        connection_handles.borrow_mut().remove(connection_id);
                    }),
                    tq_regular,
                )
                .unwrap()
                .detach();
            }
        }
    }

    Ok(())
}

async fn clean_connections(
    config: Rc<Config>,
    connection_slab: Rc<RefCell<ConnectionHandles>>,
    server_start_instant: ServerStartInstant,
    opt_tls_config: Option<Arc<ArcSwap<RustlsConfig>>>,
) -> Option<Duration> {
    let now = server_start_instant.seconds_elapsed();
    let opt_current_tls_config = opt_tls_config.map(|c| c.load_full());

    connection_slab.borrow_mut().retain(|_, reference| {
        let mut keep = true;

        // Handle case when connection runs on an old TLS certificate
        if let Some(valid_until) = reference.valid_until_after_tls_update {
            if !valid_until.valid(now) {
                keep = false;
            }
        } else if let Some(false) = opt_current_tls_config
            .as_ref()
            .zip(reference.opt_tls_config.as_ref())
            .map(|(a, b)| Arc::ptr_eq(a, b))
        {
            reference.valid_until_after_tls_update = Some(ValidUntil::new(
                server_start_instant,
                config.cleaning.close_after_tls_update_grace_period,
            ));
        }

        keep &= reference.valid_until.borrow().valid(now);

        if keep {
            true
        } else {
            if let Err(err) = reference.close_conn_sender.try_send(()) {
                ::log::info!("couldn't tell connection to close: {:#}", err);
            }

            false
        }
    });

    #[cfg(feature = "metrics")]
    {
        ::log::info!(
            "cleaned connections in worker {}, {} references remaining",
            WORKER_INDEX.get(),
            connection_slab.borrow_mut().len()
        );

        // Increment gauges by zero to prevent them from being removed due to
        // idleness

        let worker_index = WORKER_INDEX.with(|index| index.get()).to_string();

        if config.network.address.is_ipv4() || !config.network.only_ipv6 {
            ::metrics::gauge!(
                "aquatic_active_connections",
                "ip_version" => "4",
                "worker_index" => worker_index.clone(),
            )
            .increment(0.0);
        }
        if config.network.address.is_ipv6() {
            ::metrics::gauge!(
                "aquatic_active_connections",
                "ip_version" => "6",
                "worker_index" => worker_index,
            )
            .increment(0.0);
        }
    }

    Some(Duration::from_secs(
        config.cleaning.connection_cleaning_interval,
    ))
}

async fn receive_out_messages(
    mut out_message_receiver: ConnectedReceiver<(OutMessageMeta, OutMessage)>,
    connection_references: Rc<RefCell<ConnectionHandles>>,
) {
    let connection_references = &connection_references;

    while let Some((meta, out_message)) = out_message_receiver.next().await {
        if let Some(reference) = connection_references.borrow().get(meta.connection_id) {
            match reference.out_message_sender.try_send((meta, out_message)) {
                Ok(()) => {}
                Err(GlommioError::Closed(_)) => {}
                Err(GlommioError::WouldBlock(_)) => {
                    ::log::debug!(
                        "couldn't send OutMessage over local channel to Connection, channel full"
                    );
                }
                Err(err) => {
                    ::log::debug!(
                        "couldn't send OutMessage over local channel to Connection: {:?}",
                        err
                    );
                }
            }
        }
    }
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

    ::log::info!("creating socket..");

    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
        .with_context(|| "create socket")?;

    if config.network.only_ipv6 {
        ::log::info!("setting socket to ipv6 only..");

        socket
            .set_only_v6(true)
            .with_context(|| "socket: set only ipv6")?;
    }

    ::log::info!("setting SO_REUSEPORT on socket..");

    socket
        .set_reuse_port(true)
        .with_context(|| "socket: set reuse port")?;

    ::log::info!("binding socket..");

    socket
        .bind(&config.network.address.into())
        .with_context(|| format!("socket: bind to {}", config.network.address))?;

    ::log::info!("listening on socket..");

    socket
        .listen(config.network.tcp_backlog)
        .with_context(|| format!("socket: listen {}", config.network.address))?;

    ::log::info!("running PrivilegeDropper::after_socket_creation..");

    priv_dropper.after_socket_creation()?;

    ::log::info!("casting socket to glommio TcpListener..");

    Ok(unsafe { TcpListener::from_raw_fd(socket.into_raw_fd()) })
}

#[cfg(feature = "metrics")]
fn ip_version_to_metrics_str(ip_version: IpVersion) -> &'static str {
    match ip_version {
        IpVersion::V4 => "4",
        IpVersion::V6 => "6",
    }
}

fn calculate_in_message_consumer_index(config: &Config, info_hash: InfoHash) -> usize {
    (info_hash.0[0] as usize) % config.swarm_workers
}
