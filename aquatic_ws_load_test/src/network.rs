use std::{
    cell::RefCell,
    convert::TryInto,
    rc::Rc,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use aquatic_ws_protocol::{InMessage, JsonValue, OfferId, OutMessage, PeerId};
use async_tungstenite::{WebSocketStream, client_async};
use futures::{StreamExt, SinkExt};
use futures_rustls::{TlsConnector, client::TlsStream};
use glommio::net::TcpStream;
use glommio::{prelude::*, timer::TimerActionRepeat};
use rand::{Rng, SeedableRng, prelude::SmallRng};

use crate::{common::LoadTestState, config::Config, utils::create_random_request};

pub async fn run_socket_thread(
    config: Config,
    tls_config: Arc<rustls::ClientConfig>,
    load_test_state: LoadTestState,
) -> anyhow::Result<()> {
    let config = Rc::new(config);
    let num_active_connections = Rc::new(RefCell::new(0usize));

    TimerActionRepeat::repeat(move || {
        periodically_open_connections(
            config.clone(),
            tls_config.clone(),
            load_test_state.clone(),
            num_active_connections.clone(),
        )
    });

    futures::future::pending::<bool>().await;

    Ok(())
}

async fn periodically_open_connections(
    config: Rc<Config>,
    tls_config: Arc<rustls::ClientConfig>,
    load_test_state: LoadTestState,
    num_active_connections: Rc<RefCell<usize>>,
) -> Option<Duration> {
    if *num_active_connections.borrow() < config.num_connections {
        spawn_local(async move {
            if let Err(err) =
                Connection::run(config, tls_config, load_test_state, num_active_connections).await
            {
                eprintln!("connection creation error: {:?}", err);
            }
        })
        .detach();
    }

    Some(Duration::from_secs(1))
}

struct Connection {
    config: Rc<Config>,
    load_test_state: LoadTestState,
    rng: SmallRng,
    can_send: bool,
    peer_id: PeerId,
    send_answer: Option<(PeerId, OfferId)>,
    stream: WebSocketStream<TlsStream<TcpStream>>,
}

impl Connection {
    async fn run(
        config: Rc<Config>,
        tls_config: Arc<rustls::ClientConfig>,
        load_test_state: LoadTestState,
        num_active_connections: Rc<RefCell<usize>>,
    ) -> anyhow::Result<()> {
        let mut rng = SmallRng::from_entropy();
        let peer_id = PeerId(rng.gen());
        let stream = TcpStream::connect(config.server_address)
            .await
            .map_err(|err| anyhow::anyhow!("connect: {:?}", err))?;
        let stream = TlsConnector::from(tls_config).connect("example.com".try_into().unwrap(), stream).await?;
        let request = format!(
            "ws://{}:{}",
            config.server_address.ip(),
            config.server_address.port()
        );
        let (stream, _) = client_async(request, stream).await?;

        let mut connection = Connection {
            config,
            load_test_state,
            rng,
            stream,
            can_send: true,
            peer_id,
            send_answer: None,
        };

        *num_active_connections.borrow_mut() += 1;

        println!("run connection");

        if let Err(err) = connection.run_connection_loop().await {
            eprintln!("connection error: {:?}", err);
        }

        *num_active_connections.borrow_mut() -= 1;

        Ok(())
    }

    async fn run_connection_loop(&mut self) -> anyhow::Result<()> {
        loop {
            if self.can_send {
                let request =
                    create_random_request(&self.config, &self.load_test_state, &mut self.rng, self.peer_id);

                // If self.send_answer is set and request is announce request, make
                // the request an offer answer
                let request = if let InMessage::AnnounceRequest(mut r) = request {
                    if let Some((peer_id, offer_id)) = self.send_answer {
                        r.to_peer_id = Some(peer_id);
                        r.offer_id = Some(offer_id);
                        r.answer = Some(JsonValue(::serde_json::json!(
                            {"sdp": "abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-abcdefg-"}
                        )));
                        r.event = None;
                        r.offers = None;
                    }

                    self.send_answer = None;

                    InMessage::AnnounceRequest(r)
                } else {
                    request
                };

                self.stream.send(request.to_ws_message()).await?;
                self.stream.flush().await?;

                self.load_test_state
                    .statistics
                    .requests
                    .fetch_add(1, Ordering::SeqCst);

                self.can_send = false;
            }

            self.read_message().await?;
        }
    }

    async fn read_message(&mut self) -> anyhow::Result<()> {
        match OutMessage::from_ws_message(self.stream.next().await.unwrap()?) {
            Ok(OutMessage::Offer(offer)) => {
                self.load_test_state
                    .statistics
                    .responses_offer
                    .fetch_add(1, Ordering::SeqCst);

                self.send_answer = Some((offer.peer_id, offer.offer_id));

                self.can_send = true;
            }
            Ok(OutMessage::Answer(_)) => {
                self.load_test_state
                    .statistics
                    .responses_answer
                    .fetch_add(1, Ordering::SeqCst);

                self.can_send = true;
            }
            Ok(OutMessage::AnnounceResponse(_)) => {
                self.load_test_state
                    .statistics
                    .responses_announce
                    .fetch_add(1, Ordering::SeqCst);

                self.can_send = true;
            }
            Ok(OutMessage::ScrapeResponse(_)) => {
                self.load_test_state
                    .statistics
                    .responses_scrape
                    .fetch_add(1, Ordering::SeqCst);

                self.can_send = true;
            }
            Ok(OutMessage::ErrorResponse(response)) => {
                self.load_test_state
                    .statistics
                    .responses_error
                    .fetch_add(1, Ordering::SeqCst);

                eprintln!("received error response: {:?}", response.failure_reason);

                self.can_send = true;
            }
            Err(err) => {
                eprintln!("error deserializing offer: {:?}", err);
            }
        }

        Ok(())
    }
}
