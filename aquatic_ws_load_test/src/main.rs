use std::sync::{atomic::Ordering, Arc};
use std::thread;
use std::time::{Duration, Instant};

use aquatic_common::cpu_pinning::glommio::{get_worker_placement, set_affinity_for_util_worker};
use aquatic_common::cpu_pinning::WorkerIndex;
use glommio::LocalExecutorBuilder;
use rand::prelude::*;
use rand_distr::Gamma;

mod common;
mod config;
mod network;
mod utils;

use common::*;
use config::*;
use network::*;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn main() {
    aquatic_common::cli::run_app_with_cli_and_config::<Config>(
        "aquatic_ws_load_test: WebTorrent load tester",
        env!("CARGO_PKG_VERSION"),
        run,
        None,
    )
}

fn run(config: Config) -> ::anyhow::Result<()> {
    if config.torrents.weight_announce + config.torrents.weight_scrape == 0 {
        panic!("Error: at least one weight must be larger than zero.");
    }

    println!("Starting client with config: {:#?}", config);

    let mut info_hashes = Vec::with_capacity(config.torrents.number_of_torrents);

    let mut rng = SmallRng::from_entropy();

    for _ in 0..config.torrents.number_of_torrents {
        info_hashes.push(InfoHash(rng.gen()));
    }

    let gamma = Gamma::new(
        config.torrents.torrent_gamma_shape,
        config.torrents.torrent_gamma_scale,
    )
    .unwrap();

    let state = LoadTestState {
        info_hashes: Arc::new(info_hashes),
        statistics: Arc::new(Statistics::default()),
        gamma: Arc::new(gamma),
    };

    let tls_config = create_tls_config().unwrap();

    for i in 0..config.num_workers {
        let config = config.clone();
        let tls_config = tls_config.clone();
        let state = state.clone();

        let placement = get_worker_placement(
            &config.cpu_pinning,
            config.num_workers,
            0,
            WorkerIndex::SocketWorker(i),
        )?;

        LocalExecutorBuilder::new(placement)
            .name("load-test")
            .spawn(move || async move {
                run_socket_thread(config, tls_config, state).await.unwrap();
            })
            .unwrap();
    }

    if config.cpu_pinning.active {
        set_affinity_for_util_worker(&config.cpu_pinning, config.num_workers, 0)?;
    }

    monitor_statistics(state, &config);

    Ok(())
}

struct FakeCertificateVerifier;

impl rustls::client::ServerCertVerifier for FakeCertificateVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

fn create_tls_config() -> anyhow::Result<Arc<rustls::ClientConfig>> {
    let mut config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();

    config
        .dangerous()
        .set_certificate_verifier(Arc::new(FakeCertificateVerifier));

    Ok(Arc::new(config))
}

fn monitor_statistics(state: LoadTestState, config: &Config) {
    let start_time = Instant::now();
    let mut report_avg_response_vec: Vec<f64> = Vec::new();

    let interval = 5;
    let interval_f64 = interval as f64;

    loop {
        thread::sleep(Duration::from_secs(interval));

        let statistics = state.statistics.as_ref();

        let responses_announce =
            statistics.responses_announce.fetch_and(0, Ordering::Relaxed) as f64;
        // let response_peers = statistics.response_peers
        //     .fetch_and(0, Ordering::Relaxed) as f64;

        let requests_per_second =
            statistics.requests.fetch_and(0, Ordering::Relaxed) as f64 / interval_f64;
        let responses_offer_per_second =
            statistics.responses_offer.fetch_and(0, Ordering::Relaxed) as f64 / interval_f64;
        let responses_answer_per_second =
            statistics.responses_answer.fetch_and(0, Ordering::Relaxed) as f64 / interval_f64;
        let responses_scrape_per_second =
            statistics.responses_scrape.fetch_and(0, Ordering::Relaxed) as f64 / interval_f64;
        let responses_error_per_second =
            statistics.responses_error.fetch_and(0, Ordering::Relaxed) as f64 / interval_f64;

        let responses_announce_per_second = responses_announce / interval_f64;

        let connections = statistics.connections.load(Ordering::Relaxed);

        let responses_per_second = responses_announce_per_second
            + responses_offer_per_second
            + responses_answer_per_second
            + responses_scrape_per_second
            + responses_error_per_second;

        report_avg_response_vec.push(responses_per_second);

        println!();
        println!("Requests out: {:.2}/second", requests_per_second);
        println!("Responses in: {:.2}/second", responses_per_second);
        println!(
            "  - Announce responses: {:.2}",
            responses_announce_per_second
        );
        println!("  - Offer responses:    {:.2}", responses_offer_per_second);
        println!("  - Answer responses:   {:.2}", responses_answer_per_second);
        println!("  - Scrape responses:   {:.2}", responses_scrape_per_second);
        println!("  - Error responses:   {:.2}", responses_error_per_second);
        println!("Active connections: {}", connections);

        let time_elapsed = start_time.elapsed();
        let duration = Duration::from_secs(config.duration as u64);

        if config.duration != 0 && time_elapsed >= duration {
            let report_len = report_avg_response_vec.len() as f64;
            let report_sum: f64 = report_avg_response_vec.into_iter().sum();
            let report_avg: f64 = report_sum / report_len;

            println!(
                concat!(
                    "\n# aquatic load test report\n\n",
                    "Test ran for {} seconds.\n",
                    "Average responses per second: {:.2}\n\nConfig: {:#?}\n"
                ),
                time_elapsed.as_secs(),
                report_avg,
                config
            );

            break;
        }
    }
}
