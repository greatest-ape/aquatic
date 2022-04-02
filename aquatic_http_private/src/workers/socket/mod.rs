mod db;
mod routes;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};

use anyhow::Context;
use axum::{routing::get, Extension, Router};
use sqlx::mysql::MySqlPoolOptions;

pub fn run_socket_worker() -> anyhow::Result<()> {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 3000));

    let tcp_listener = create_tcp_listener(addr, false)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_app(tcp_listener))?;

    Ok(())
}

async fn run_app(tcp_listener: TcpListener) -> anyhow::Result<()> {
    let db_url = ::std::env::var("DATABASE_URL").unwrap();

    let pool = MySqlPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    let app = Router::new()
        .route("/:user_token/announce/", get(routes::announce))
        .layer(Extension(pool));

    axum::Server::from_tcp(tcp_listener)?
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn create_tcp_listener(addr: SocketAddr, only_ipv6: bool) -> anyhow::Result<TcpListener> {
    let domain = if addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };

    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;

    if only_ipv6 {
        socket.set_only_v6(true).with_context(|| "set only_ipv6")?;
    }

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
