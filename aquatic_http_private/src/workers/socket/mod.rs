pub mod db;
mod routes;
mod tls;

use std::{
    net::{SocketAddr, TcpListener},
    sync::Arc,
};

use anyhow::Context;
use aquatic_common::{rustls_config::RustlsConfig, PanicSentinel};
use axum::{extract::connect_info::Connected, routing::get, Extension, Router};
use hyper::server::conn::AddrIncoming;
use sqlx::mysql::MySqlPoolOptions;

use self::tls::{TlsAcceptor, TlsStream};
use crate::{common::ChannelRequestSender, config::Config};

impl<'a> Connected<&'a tls::TlsStream> for SocketAddr {
    fn connect_info(target: &'a TlsStream) -> Self {
        target.get_remote_addr()
    }
}

pub fn run_socket_worker(
    _sentinel: PanicSentinel,
    config: Config,
    tls_config: Arc<RustlsConfig>,
    request_sender: ChannelRequestSender,
) -> anyhow::Result<()> {
    let tcp_listener = create_tcp_listener(config.network.address)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_app(config, tls_config, tcp_listener, request_sender))?;

    Ok(())
}

async fn run_app(
    config: Config,
    tls_config: Arc<RustlsConfig>,
    tcp_listener: TcpListener,
    request_sender: ChannelRequestSender,
) -> anyhow::Result<()> {
    let db_url =
        ::std::env::var("DATABASE_URL").with_context(|| "Retrieve env var DATABASE_URL")?;

    let tls_acceptor = TlsAcceptor::new(
        tls_config,
        AddrIncoming::from_listener(tokio::net::TcpListener::from_std(tcp_listener)?)?,
    );

    let pool = MySqlPoolOptions::new()
        .max_connections(config.db_connections_per_worker)
        .connect(&db_url)
        .await?;

    let app = Router::new()
        .route("/announce/:user_token/", get(routes::announce))
        .layer(Extension(Arc::new(config.clone())))
        .layer(Extension(pool))
        .layer(Extension(Arc::new(request_sender)));

    axum::Server::builder(tls_acceptor)
        .http1_keepalive(config.network.keep_alive)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;

    Ok(())
}

fn create_tcp_listener(addr: SocketAddr) -> anyhow::Result<TcpListener> {
    let domain = if addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;

    socket
        .set_reuse_port(true)
        .with_context(|| "set_reuse_port")?;
    socket
        .set_nonblocking(true)
        .with_context(|| "set_nonblocking")?;
    socket
        .bind(&addr.into())
        .with_context(|| format!("bind to {}", addr))?;
    socket
        .listen(1024)
        .with_context(|| format!("listen on {}", addr))?;

    Ok(socket.into())
}
