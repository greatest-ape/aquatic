pub mod db;
mod routes;

use std::{
    net::{SocketAddr, TcpListener},
    sync::Arc,
};

use anyhow::Context;
use axum::{routing::get, Extension, Router};
use sqlx::mysql::MySqlPoolOptions;

use crate::{common::ChannelRequestSender, config::Config};

pub fn run_socket_worker(
    config: Config,
    request_sender: ChannelRequestSender,
) -> anyhow::Result<()> {
    let tcp_listener = create_tcp_listener(config.network.address)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_app(config, tcp_listener, request_sender))?;

    Ok(())
}

async fn run_app(
    config: Config,
    tcp_listener: TcpListener,
    request_sender: ChannelRequestSender,
) -> anyhow::Result<()> {
    let db_url =
        ::std::env::var("DATABASE_URL").with_context(|| "Retrieve env var DATABASE_URL")?;

    let pool = MySqlPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    let app = Router::new()
        .route("/announce/:user_token/", get(routes::announce))
        .layer(Extension(Arc::new(config)))
        .layer(Extension(pool))
        .layer(Extension(Arc::new(request_sender)));

    axum::Server::from_tcp(tcp_listener)?
        .http1_keepalive(false)
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
        .bind(&addr.into())
        .with_context(|| format!("bind to {}", addr))?;
    socket
        .listen(1024)
        .with_context(|| format!("listen on {}", addr))?;

    Ok(socket.into())
}
