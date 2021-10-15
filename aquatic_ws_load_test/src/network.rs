use std::io::ErrorKind;
use std::sync::atomic::Ordering;
use std::time::Duration;

use hashbrown::HashMap;
use mio::{net::TcpStream, Events, Interest, Poll, Token};
use rand::{prelude::*, rngs::SmallRng};
use tungstenite::{handshake::MidHandshake, ClientHandshake, HandshakeError, WebSocket};

use crate::common::*;
use crate::config::*;
use crate::utils::create_random_request;

// Allow large enum variant WebSocket because it should be very common
#[allow(clippy::large_enum_variant)]
pub enum ConnectionState {
    TcpStream(TcpStream),
    WebSocket(WebSocket<TcpStream>),
    MidHandshake(MidHandshake<ClientHandshake<TcpStream>>),
}

impl ConnectionState {
    fn advance(self, config: &Config) -> Option<Self> {
        match self {
            Self::TcpStream(stream) => {
                let req = format!(
                    "ws://{}:{}",
                    config.server_address.ip(),
                    config.server_address.port()
                );

                match ::tungstenite::client(req, stream) {
                    Ok((ws, _)) => Some(ConnectionState::WebSocket(ws)),
                    Err(HandshakeError::Interrupted(handshake)) => {
                        Some(ConnectionState::MidHandshake(handshake))
                    }
                    Err(HandshakeError::Failure(err)) => {
                        eprintln!("handshake error: {:?}", err);

                        None
                    }
                }
            }
            Self::MidHandshake(handshake) => match handshake.handshake() {
                Ok((ws, _)) => Some(ConnectionState::WebSocket(ws)),
                Err(HandshakeError::Interrupted(handshake)) => {
                    Some(ConnectionState::MidHandshake(handshake))
                }
                Err(HandshakeError::Failure(err)) => {
                    eprintln!("handshake error: {:?}", err);

                    None
                }
            },
            Self::WebSocket(ws) => Some(Self::WebSocket(ws)),
        }
    }
}

pub struct Connection {
    stream: ConnectionState,
    peer_id: PeerId,
    can_send: bool,
    send_answer: Option<(PeerId, OfferId)>,
}

impl Connection {
    pub fn create_and_register(
        config: &Config,
        rng: &mut impl Rng,
        connections: &mut ConnectionMap,
        poll: &mut Poll,
        token_counter: &mut usize,
    ) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect(config.server_address)?;

        poll.registry()
            .register(
                &mut stream,
                Token(*token_counter),
                Interest::READABLE | Interest::WRITABLE,
            )
            .unwrap();

        let connection = Connection {
            stream: ConnectionState::TcpStream(stream),
            peer_id: PeerId(rng.gen()),
            can_send: false,
            send_answer: None,
        };

        connections.insert(*token_counter, connection);

        *token_counter += 1;

        Ok(())
    }

    pub fn advance(self, config: &Config) -> Option<Self> {
        if let Some(stream) = self.stream.advance(config) {
            let can_send = matches!(stream, ConnectionState::WebSocket(_));

            Some(Self {
                stream,
                peer_id: self.peer_id,
                can_send,
                send_answer: None,
            })
        } else {
            None
        }
    }

    pub fn read_responses(&mut self, state: &LoadTestState) -> bool {
        // bool = drop connection
        if let ConnectionState::WebSocket(ref mut ws) = self.stream {
            loop {
                match ws.read_message() {
                    Ok(message) => match OutMessage::from_ws_message(message) {
                        Ok(OutMessage::Offer(offer)) => {
                            state
                                .statistics
                                .responses_offer
                                .fetch_add(1, Ordering::SeqCst);

                            self.send_answer = Some((offer.peer_id, offer.offer_id));

                            self.can_send = true;
                        }
                        Ok(OutMessage::Answer(_)) => {
                            state
                                .statistics
                                .responses_answer
                                .fetch_add(1, Ordering::SeqCst);

                            self.can_send = true;
                        }
                        Ok(OutMessage::AnnounceResponse(_)) => {
                            state
                                .statistics
                                .responses_announce
                                .fetch_add(1, Ordering::SeqCst);

                            self.can_send = true;
                        }
                        Ok(OutMessage::ScrapeResponse(_)) => {
                            state
                                .statistics
                                .responses_scrape
                                .fetch_add(1, Ordering::SeqCst);

                            self.can_send = true;
                        }
                        Ok(OutMessage::ErrorResponse(response)) => {
                            state
                                .statistics
                                .responses_error
                                .fetch_add(1, Ordering::SeqCst);
                            
                            eprintln!("received error response: {:?}", response.failure_reason);

                            self.can_send = true;
                        },
                        Err(err) => {
                            eprintln!("error deserializing offer: {:?}", err);
                        }
                    },
                    Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock => {
                        return false;
                    }
                    Err(_) => {
                        return true;
                    }
                }
            }
        }

        false
    }

    pub fn send_request(
        &mut self,
        config: &Config,
        state: &LoadTestState,
        rng: &mut impl Rng,
    ) -> bool {
        // bool = remove connection
        if !self.can_send {
            return false;
        }

        if let ConnectionState::WebSocket(ref mut ws) = self.stream {
            let request = create_random_request(&config, &state, rng, self.peer_id);

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

            match ws.write_message(request.to_ws_message()) {
                Ok(()) => {
                    state.statistics.requests.fetch_add(1, Ordering::SeqCst);

                    self.can_send = false;

                    false
                }
                Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock => false,
                Err(_) => true,
            }
        } else {
            println!("send request can't send to non-ws stream");

            false
        }
    }
}

pub type ConnectionMap = HashMap<usize, Connection>;

pub fn run_socket_thread(config: &Config, state: LoadTestState) {
    let timeout = Duration::from_micros(config.network.poll_timeout_microseconds);
    let create_conn_interval = 2 ^ config.network.connection_creation_interval;

    let mut connections: ConnectionMap = HashMap::with_capacity(config.num_connections);
    let mut poll = Poll::new().expect("create poll");
    let mut events = Events::with_capacity(config.network.poll_event_capacity);
    let mut rng = SmallRng::from_entropy();

    let mut token_counter = 0usize;
    let mut iter_counter = 0usize;

    let mut drop_keys = Vec::new();

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter() {
            let token = event.token();

            if event.is_readable() {
                if let Some(connection) = connections.get_mut(&token.0) {
                    if let ConnectionState::WebSocket(_) = connection.stream {
                        let drop_connection = connection.read_responses(&state);

                        if drop_connection {
                            connections.remove(&token.0);
                        }

                        continue;
                    }
                }
            }

            if let Some(connection) = connections.remove(&token.0) {
                if let Some(connection) = connection.advance(config) {
                    connections.insert(token.0, connection);
                }
            }
        }

        for (k, connection) in connections.iter_mut() {
            let drop_connection = connection.send_request(config, &state, &mut rng);

            if drop_connection {
                drop_keys.push(*k)
            }
        }

        for k in drop_keys.drain(..) {
            connections.remove(&k);
        }

        // Slowly create new connections
        if connections.len() < config.num_connections && iter_counter % create_conn_interval == 0 {
            let res = Connection::create_and_register(
                config,
                &mut rng,
                &mut connections,
                &mut poll,
                &mut token_counter,
            );

            if let Err(err) = res {
                eprintln!("create connection error: {}", err);
            }
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}
