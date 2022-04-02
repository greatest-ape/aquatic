pub mod config;
mod workers;

use dotenv::dotenv;

pub const APP_NAME: &str = "aquatic_http_private: private HTTP/TLS BitTorrent tracker";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run(config: config::Config) -> anyhow::Result<()> {
    dotenv().ok();

    let mut handles = Vec::new();

    for _ in 0..config.socket_workers {
        let config = config.clone();

        let handle = ::std::thread::Builder::new()
            .name("socket".into())
            .spawn(move || workers::socket::run_socket_worker(config))?;

        handles.push(handle);
    }

    for _ in 0..config.request_workers {
        let config = config.clone();

        let handle = ::std::thread::Builder::new()
            .name("request".into())
            .spawn(move || workers::request::run_request_worker(config))?;

        handles.push(handle);
    }

    for handle in handles {
        handle
            .join()
            .map_err(|err| anyhow::anyhow!("thread join error: {:?}", err))??;
    }

    Ok(())
}
