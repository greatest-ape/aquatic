use std::{net::SocketAddr, sync::Arc};

use axum::{extract::State, http::StatusCode, response::Html, routing::get, Json, Router, Server};
use flume::{Receiver, Sender};
use serde::Serialize;
use tokio::runtime::Builder;
use tower_http::compression::CompressionLayer;

use crate::PanicSentinel;

const INDEX_PAGE: &str =
    r#"<h1>aquatic api</h1><ul><li><a href="/full-scrape">/full-scrape</a> (JSON)</li></ul>"#;

pub type FullScrapeRequestReceiver = Receiver<FullScrapeRequest>;

#[derive(Clone)]
pub struct FullScrapeRequest {
    pub response_sender: Sender<FullScrapeResponse>,
}

#[derive(Clone, Serialize)]
pub struct FullScrapeResponse {
    pub ipv4: Vec<FullScrapeStatistics>,
    pub ipv6: Vec<FullScrapeStatistics>,
}

#[derive(Clone, Copy, Serialize)]
pub struct FullScrapeStatistics {
    #[serde(with = "hex::serde")]
    pub info_hash: [u8; 20],
    pub seeders: usize,
    pub leechers: usize,
}

pub struct FullScrapeWorker {
    addr: SocketAddr,
    swarm_message_senders: Arc<[Sender<FullScrapeRequest>]>,
}

impl FullScrapeWorker {
    pub fn new(
        addr: SocketAddr,
        num_swarm_workers: usize,
    ) -> (Self, Vec<Receiver<FullScrapeRequest>>) {
        let mut senders = Vec::new();
        let mut receivers = Vec::new();

        for _ in 0..num_swarm_workers {
            let (sender, receiver) = flume::unbounded();

            senders.push(sender);
            receivers.push(receiver);
        }

        let worker = Self {
            addr,
            swarm_message_senders: senders.into(),
        };

        (worker, receivers)
    }

    pub fn run(self, _sentinel: PanicSentinel) -> anyhow::Result<()> {
        let runtime = Builder::new_current_thread().enable_all().build()?;

        runtime.block_on(async { self.run_inner().await })?;

        Ok(())
    }

    async fn run_inner(self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/", get(|| async { Html(INDEX_PAGE) }))
            .route("/full-scrape", get(full_scrape_route))
            .layer(CompressionLayer::new())
            .with_state(self.swarm_message_senders);

        Server::bind(&self.addr)
            .serve(app.into_make_service())
            .await?;

        Ok(())
    }
}

async fn full_scrape_route(
    State(request_senders): State<Arc<[Sender<FullScrapeRequest>]>>,
) -> Result<Json<FullScrapeResponse>, StatusCode> {
    let num_swarm_workers = request_senders.len();

    let (response_sender, response_receiver) = flume::bounded(num_swarm_workers);

    for request_sender in request_senders.iter() {
        let message = FullScrapeRequest {
            response_sender: response_sender.clone(),
        };

        if let Err(_) = request_sender.send(message) {
            ::log::error!("full scrape: request channel full");

            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    let mut full = FullScrapeResponse {
        ipv4: Vec::new(),
        ipv6: Vec::new(),
    };

    for _ in 0..num_swarm_workers {
        match response_receiver.recv_async().await {
            Ok(mut response) => {
                full.ipv4.append(&mut response.ipv4);
                full.ipv6.append(&mut response.ipv6);
            }
            Err(_) => {
                ::log::error!("full scrape: response sender dropped too early");

                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }

    Ok(Json(full))
}
