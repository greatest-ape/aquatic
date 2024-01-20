use std::{
    cell::RefCell,
    convert::TryInto,
    rc::Rc,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use aquatic_ws_protocol::incoming::{
    AnnounceEvent, AnnounceRequest, AnnounceRequestOffer, InMessage, ScrapeRequestInfoHashes,
};
use aquatic_ws_protocol::outgoing::OutMessage;
use aquatic_ws_protocol::{
    common::{
        AnnounceAction, InfoHash, OfferId, PeerId, RtcAnswer, RtcAnswerType, RtcOffer,
        RtcOfferType, ScrapeAction,
    },
    incoming::ScrapeRequest,
};
use async_tungstenite::{client_async, WebSocketStream};
use futures::{SinkExt, StreamExt};
use futures_rustls::{client::TlsStream, TlsConnector};
use glommio::net::TcpStream;
use glommio::{prelude::*, timer::TimerActionRepeat};
use rand::{prelude::SmallRng, Rng, SeedableRng};
use rand_distr::{Distribution, WeightedIndex};

use crate::{
    common::{LoadTestState, RequestType},
    config::Config,
    utils::select_info_hash_index,
};

pub async fn run_socket_thread(
    config: Config,
    tls_config: Arc<rustls::ClientConfig>,
    load_test_state: LoadTestState,
) -> anyhow::Result<()> {
    let config = Rc::new(config);
    let rng = Rc::new(RefCell::new(SmallRng::from_entropy()));
    let num_active_connections = Rc::new(RefCell::new(0usize));
    let connection_creation_interval =
        Duration::from_millis(config.connection_creation_interval_ms);

    TimerActionRepeat::repeat(move || {
        periodically_open_connections(
            config.clone(),
            tls_config.clone(),
            load_test_state.clone(),
            num_active_connections.clone(),
            rng.clone(),
            connection_creation_interval,
        )
    })
    .join()
    .await;

    Ok(())
}

async fn periodically_open_connections(
    config: Rc<Config>,
    tls_config: Arc<rustls::ClientConfig>,
    load_test_state: LoadTestState,
    num_active_connections: Rc<RefCell<usize>>,
    rng: Rc<RefCell<SmallRng>>,
    connection_creation_interval: Duration,
) -> Option<Duration> {
    if *num_active_connections.borrow() < config.num_connections_per_worker {
        spawn_local(async move {
            if let Err(err) = Connection::run(
                config,
                tls_config,
                load_test_state,
                num_active_connections,
                rng,
            )
            .await
            {
                ::log::info!("connection creation error: {:#}", err);
            }
        })
        .detach();
    }

    Some(connection_creation_interval)
}

struct Connection {
    config: Rc<Config>,
    load_test_state: LoadTestState,
    rng: Rc<RefCell<SmallRng>>,
    peer_id: PeerId,
    can_send_answer: Option<(InfoHash, PeerId, OfferId)>,
    stream: WebSocketStream<TlsStream<TcpStream>>,
}

impl Connection {
    async fn run(
        config: Rc<Config>,
        tls_config: Arc<rustls::ClientConfig>,
        load_test_state: LoadTestState,
        num_active_connections: Rc<RefCell<usize>>,
        rng: Rc<RefCell<SmallRng>>,
    ) -> anyhow::Result<()> {
        let peer_id = PeerId(rng.borrow_mut().gen());
        let stream = TcpStream::connect(config.server_address)
            .await
            .map_err(|err| anyhow::anyhow!("connect: {:?}", err))?;
        let stream = TlsConnector::from(tls_config)
            .connect("example.com".try_into().unwrap(), stream)
            .await?;
        let request = format!(
            "ws://{}:{}",
            config.server_address.ip(),
            config.server_address.port()
        );
        let (stream, _) = client_async(request, stream).await?;

        let statistics = load_test_state.statistics.clone();

        let mut connection = Connection {
            config,
            load_test_state,
            rng,
            stream,
            peer_id,
            can_send_answer: None,
        };

        *num_active_connections.borrow_mut() += 1;
        statistics.connections.fetch_add(1, Ordering::Relaxed);

        if let Err(err) = connection.run_connection_loop().await {
            ::log::info!("connection error: {:#}", err);
        }

        *num_active_connections.borrow_mut() -= 1;
        statistics.connections.fetch_sub(1, Ordering::Relaxed);

        Ok(())
    }

    async fn run_connection_loop(&mut self) -> anyhow::Result<()> {
        loop {
            self.send_message().await?;
            self.read_message().await?;
        }
    }

    async fn send_message(&mut self) -> anyhow::Result<()> {
        let request = self.create_request();

        self.stream.send(request.to_ws_message()).await?;

        self.load_test_state
            .statistics
            .requests
            .fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    fn create_request(&mut self) -> InMessage {
        let mut rng = self.rng.borrow_mut();

        let request = match random_request_type(&self.config, &mut *rng) {
            RequestType::Announce => {
                let (event, bytes_left) = {
                    if rng.gen_bool(self.config.torrents.peer_seeder_probability) {
                        (AnnounceEvent::Completed, 0)
                    } else {
                        (AnnounceEvent::Started, 50)
                    }
                };

                const SDP: &str = "abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg";

                if let Some((info_hash, peer_id, offer_id)) = self.can_send_answer {
                    InMessage::AnnounceRequest(AnnounceRequest {
                        info_hash,
                        answer_to_peer_id: Some(peer_id),
                        answer_offer_id: Some(offer_id),
                        answer: Some(RtcAnswer {
                            t: RtcAnswerType::Answer,
                            sdp: SDP.into(),
                        }),
                        event: None,
                        offers: None,
                        action: AnnounceAction::Announce,
                        peer_id: self.peer_id,
                        bytes_left: Some(bytes_left),
                        numwant: Some(0),
                    })
                } else {
                    let info_hash_index =
                        select_info_hash_index(&self.config, &self.load_test_state, &mut *rng);

                    let mut offers = Vec::with_capacity(self.config.torrents.offers_per_request);

                    for _ in 0..self.config.torrents.offers_per_request {
                        offers.push(AnnounceRequestOffer {
                            offer_id: OfferId(rng.gen()),
                            offer: RtcOffer {
                                t: RtcOfferType::Offer,
                                sdp: SDP.into(),
                            },
                        })
                    }

                    InMessage::AnnounceRequest(AnnounceRequest {
                        action: AnnounceAction::Announce,
                        info_hash: self.load_test_state.info_hashes[info_hash_index],
                        peer_id: self.peer_id,
                        bytes_left: Some(bytes_left),
                        event: Some(event),
                        numwant: Some(offers.len()),
                        offers: Some(offers),
                        answer: None,
                        answer_to_peer_id: None,
                        answer_offer_id: None,
                    })
                }
            }
            RequestType::Scrape => {
                let mut scrape_hashes = Vec::with_capacity(5);

                for _ in 0..5 {
                    let info_hash_index =
                        select_info_hash_index(&self.config, &self.load_test_state, &mut *rng);

                    scrape_hashes.push(self.load_test_state.info_hashes[info_hash_index]);
                }

                InMessage::ScrapeRequest(ScrapeRequest {
                    action: ScrapeAction::Scrape,
                    info_hashes: Some(ScrapeRequestInfoHashes::Multiple(scrape_hashes)),
                })
            }
        };

        self.can_send_answer = None;

        request
    }

    async fn read_message(&mut self) -> anyhow::Result<()> {
        let message = match self
            .stream
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("stream finished"))??
        {
            message @ tungstenite::Message::Text(_) | message @ tungstenite::Message::Binary(_) => {
                message
            }
            message => {
                ::log::warn!(
                    "Received WebSocket message of unexpected type: {:?}",
                    message
                );

                return Ok(());
            }
        };

        match OutMessage::from_ws_message(message) {
            Ok(OutMessage::OfferOutMessage(offer)) => {
                self.load_test_state
                    .statistics
                    .responses_offer
                    .fetch_add(1, Ordering::Relaxed);

                self.can_send_answer = Some((offer.info_hash, offer.peer_id, offer.offer_id));
            }
            Ok(OutMessage::AnswerOutMessage(_)) => {
                self.load_test_state
                    .statistics
                    .responses_answer
                    .fetch_add(1, Ordering::Relaxed);
            }
            Ok(OutMessage::AnnounceResponse(_)) => {
                self.load_test_state
                    .statistics
                    .responses_announce
                    .fetch_add(1, Ordering::Relaxed);
            }
            Ok(OutMessage::ScrapeResponse(_)) => {
                self.load_test_state
                    .statistics
                    .responses_scrape
                    .fetch_add(1, Ordering::Relaxed);
            }
            Ok(OutMessage::ErrorResponse(response)) => {
                self.load_test_state
                    .statistics
                    .responses_error
                    .fetch_add(1, Ordering::Relaxed);

                ::log::warn!("received error response: {:?}", response.failure_reason);
            }
            Err(err) => {
                ::log::error!("error deserializing message: {:#}", err);
            }
        }

        Ok(())
    }
}

pub fn random_request_type(config: &Config, rng: &mut impl Rng) -> RequestType {
    let weights = [
        config.torrents.weight_announce as u32,
        config.torrents.weight_scrape as u32,
    ];

    let items = [RequestType::Announce, RequestType::Scrape];

    let dist = WeightedIndex::new(weights).expect("random request weighted index");

    items[dist.sample(rng)]
}
