use std::sync::{atomic::Ordering, Arc};
use std::thread;
use std::time::{Duration, Instant};

use aquatic_ws_protocol::common::InfoHash;
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

    let mut rng = SmallRng::from_entropy();

    let mut info_hashes = Vec::with_capacity(config.torrents.number_of_torrents);

    for _ in 0..config.torrents.number_of_torrents {
        info_hashes.push(InfoHash(rng.gen()));
    }

    let gamma = Gamma::new(
        config.torrents.torrent_gamma_shape,
        config.torrents.torrent_gamma_scale,
    )
    .unwrap();

    let state = LoadTestState {
        info_hashes: Arc::from(info_hashes.into_boxed_slice()),
        statistics: Arc::new(Statistics::default()),
        gamma: Arc::new(gamma),
    };

    let tls_config = create_tls_config().unwrap();

    for _ in 0..config.num_workers {
        let config = config.clone();
        let tls_config = tls_config.clone();
        let state = state.clone();

        LocalExecutorBuilder::default()
            .name("load-test")
            .spawn(move || async move {
                run_socket_thread(config, tls_config, state).await.unwrap();
            })
            .unwrap();
    }

    monitor_statistics(state, &config);

    Ok(())
}

#[derive(Debug)]
struct FakeCertificateVerifier;

impl rustls::client::danger::ServerCertVerifier for FakeCertificateVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

fn create_tls_config() -> anyhow::Result<Arc<rustls::ClientConfig>> {
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();

    config
        .dangerous()
        .set_certificate_verifier(Arc::new(FakeCertificateVerifier));

    Ok(Arc::new(config))
}

fn monitor_statistics(state: LoadTestState, config: &Config) {
    let start_time = Instant::now();
    let mut time_max_connections_reached = None;
    let mut report_avg_response_vec: Vec<f64> = Vec::new();

    let interval = 5;
    let interval_f64 = interval as f64;

    loop {
        thread::sleep(Duration::from_secs(interval));

        let statistics = state.statistics.as_ref();

        let responses_announce = statistics
            .responses_announce
            .fetch_and(0, Ordering::Relaxed) as f64;
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

        if !config.measure_after_max_connections_reached || time_max_connections_reached.is_some() {
            report_avg_response_vec.push(responses_per_second);
        } else if connections >= config.num_workers * config.num_connections_per_worker {
            time_max_connections_reached = Some(Instant::now());

            println!();
            println!("Max connections reached");
            println!();
        }

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

        if config.measure_after_max_connections_reached {
            if let Some(start) = time_max_connections_reached {
                let time_elapsed = start.elapsed();

                if config.duration != 0
                    && time_elapsed >= Duration::from_secs(config.duration as u64)
                {
                    report(config, report_avg_response_vec, time_elapsed);

                    break;
                }
            }
        } else {
            let time_elapsed = start_time.elapsed();

            if config.duration != 0 && time_elapsed >= Duration::from_secs(config.duration as u64) {
                report(config, report_avg_response_vec, time_elapsed);

                break;
            }
        }
    }
}

fn report(config: &Config, report_avg_response_vec: Vec<f64>, time_elapsed: Duration) {
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
}
