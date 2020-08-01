use std::sync::atomic::Ordering;
use std::time::Duration;
use std::io::ErrorKind;

use hashbrown::HashMap;
use mio::{net::TcpStream, Events, Poll, Interest, Token};
use rand::{rngs::SmallRng, prelude::*};
use tungstenite::{WebSocket, HandshakeError, ClientHandshake, handshake::MidHandshake};

use crate::common::*;
use crate::config::*;
use crate::utils::create_random_request;


pub enum ConnectionState {
    TcpStream(TcpStream),
    WebSocket(WebSocket<TcpStream>),
    MidHandshake(MidHandshake<ClientHandshake<TcpStream>>)
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

                match ::tungstenite::client(req, stream){
                    Ok((ws, _)) => {
                        Some(ConnectionState::WebSocket(ws))
                    },
                    Err(HandshakeError::Interrupted(handshake)) => {
                        Some(ConnectionState::MidHandshake(handshake))
                    },
                    Err(HandshakeError::Failure(err)) => {
                        eprintln!("handshake error: {:?}", err);

                        None
                    }
                }
            },
            Self::MidHandshake(handshake) => {
                match handshake.handshake() {
                    Ok((ws, _)) => {
                        Some(ConnectionState::WebSocket(ws))
                    },
                    Err(HandshakeError::Interrupted(handshake)) => {
                        Some(ConnectionState::MidHandshake(handshake))
                    },
                    Err(HandshakeError::Failure(err)) => {
                        eprintln!("handshake error: {:?}", err);

                        None
                    }
                }
            },
            Self::WebSocket(ws) => Some(Self::WebSocket(ws)),
        }
    }

}


pub struct Connection {
    stream: ConnectionState,
    can_send_initial: bool,
    marked_as_complete: bool,
    send_answer: Option<(PeerId, OfferId)>,
}


impl Connection {
    pub fn create_and_register(
        config: &Config,
        connections: &mut ConnectionMap,
        poll: &mut Poll,
        token_counter: &mut usize,
    ) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect(config.server_address)?;
    
        poll.registry()
            .register(&mut stream, Token(*token_counter), Interest::READABLE)
            .unwrap();

        let connection  = Connection {
            stream: ConnectionState::TcpStream(stream),
            can_send_initial: false,
            marked_as_complete: false,
            send_answer: None,
        };
    
        connections.insert(*token_counter, connection);

        *token_counter += 1;
    
        Ok(())
    }

    pub fn advance(self, config: &Config) -> Option<Self> {
        if let Some(stream) = self.stream.advance(config){
            Some(Self {
                stream,
                can_send_initial: self.can_send_initial,
                marked_as_complete: false,
                send_answer: self.send_answer,
            })
        } else {
            None
        }
    }

    pub fn read_response_and_send_request(
        &mut self,
        config: &Config,
        state: &LoadTestState,
        rng: &mut impl Rng,
    ){
        if let ConnectionState::WebSocket(ref mut ws) = self.stream {
            let mut send_random_request = false;

            loop {
                match ws.read_message(){
                    Ok(message) => {
                        match OutMessage::from_ws_message(message){
                            Ok(OutMessage::Offer(offer)) => {
                                state.statistics.responses_offer
                                    .fetch_add(1, Ordering::SeqCst);
                                
                                self.send_answer = Some((
                                    offer.peer_id,
                                    offer.offer_id
                                ));

                                send_random_request = true;
                            },
                            Ok(OutMessage::Answer(_)) => {
                                state.statistics.responses_answer
                                    .fetch_add(1, Ordering::SeqCst);

                                send_random_request = true;
                            },
                            Ok(OutMessage::AnnounceResponse(_)) => {
                                state.statistics.responses_announce
                                    .fetch_add(1, Ordering::SeqCst);

                                send_random_request = true;
                            },
                            Ok(OutMessage::ScrapeResponse(_)) => {
                                state.statistics.responses_scrape
                                    .fetch_add(1, Ordering::SeqCst);

                                send_random_request = true;
                            },
                            Err(err) => {
                                eprintln!("error deserializing offer: {:?}", err);
                            }
                        }
                    },
                    Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock => {
                        self.can_send_initial = false;

                        break;
                    },
                    Err(err) => {
                        eprintln!("handle_read_event error: {}", err);

                        break;
                    }
                }
            };

            if send_random_request {
                self.send_random_request(
                    config,
                    state,
                    rng,
                );
            }
        }
    }

    pub fn send_random_request(
        &mut self,
        config: &Config,
        state: &LoadTestState,
        rng: &mut impl Rng,
    ){
        let request = create_random_request(
            &config,
            &state,
            rng
        );

        // If self.send_answer is set and request is announce request, make
        // the request an offer answer
        let request = if let InMessage::AnnounceRequest(mut r) = request {
            if let Some((peer_id, offer_id)) = self.send_answer {
                r.to_peer_id = Some(peer_id);
                r.offer_id = Some(offer_id);
                r.answer = Some(JsonValue(::serde_json::Value::from(JSON_VALUE)));
                r.offers = None;
            }

            self.send_answer = None;

            InMessage::AnnounceRequest(r)
        } else {
            request
        };

        self.send_request(state, request)
    }

    fn send_request(
        &mut self,
        state: &LoadTestState,
        request: InMessage
    ){
        if let ConnectionState::WebSocket(ref mut ws) = self.stream {
            match ws.write_message(request.to_ws_message()){
                Ok(_) => {
                    state.statistics.requests.fetch_add(1, Ordering::SeqCst);
                },
                Err(err) => {
                    eprintln!("send request error: {}", err);
                }
            }

            self.can_send_initial = false;
        } else {
            println!("send request can't send to non-ws stream");
        }
    }
}


pub type ConnectionMap = HashMap<usize, Connection>;


pub fn run_socket_thread(
    config: &Config,
    state: LoadTestState,
    num_initial_requests: usize,
) {
    let timeout = Duration::from_micros(config.network.poll_timeout_microseconds);
    let create_conn_interval = 2 ^ config.network.connection_creation_interval;

    let mut connections: ConnectionMap = HashMap::with_capacity(config.num_connections);
    let mut poll = Poll::new().expect("create poll");
    let mut events = Events::with_capacity(config.network.poll_event_capacity);
    let mut rng = SmallRng::from_entropy();

    let mut token_counter = 0usize;

    for _ in 0..num_initial_requests {
        Connection::create_and_register(
            config,
            &mut connections,
            &mut poll,
            &mut token_counter,
        ).unwrap();
    }

    println!("num connections in map: {}", connections.len());

    let mut initial_sent = false;
    let mut iter_counter = 0usize;

    let mut num_completed = 0usize;

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter(){
            if event.is_readable(){
                let token = event.token();

                let mut run_advance = false;

                if let Some(connection) = connections.get_mut(&token.0){
                    if let ConnectionState::WebSocket(_) = connection.stream {
                        connection.read_response_and_send_request(
                            config,
                            &state,
                            &mut rng,
                        );
                    } else {
                        run_advance = true;

                        println!("set run_advance=true");
                    }
                } else {
                    eprintln!("connection not found: {:?}", token);
                }

                if run_advance {
                    let connection = connections.remove(&token.0).unwrap();

                    if let Some(connection) = connection.advance(config){
                        println!("advanced connection");
                        connections.insert(token.0, connection);
                    }
                }
            }
        }

        if num_completed != token_counter {
            for k in 0..token_counter {
                if let Some(mut connection) = connections.remove(&k){
                    if let ConnectionState::WebSocket(_) = connection.stream {
                        if !connection.marked_as_complete {
                            connection.can_send_initial = true;
                            connection.marked_as_complete = true;
                            initial_sent = false;
                            num_completed += 1;
                        }

                        connections.insert(k, connection);

                    } else {
                        if let Some(connection) = connection.advance(config){
                            connections.insert(k, connection);
                        }
                    }
                } else {
                    // println!("connection not found for token {}", k);
                }
            }
        }

        if !initial_sent {
            for (_, connection) in connections.iter_mut(){
                if connection.can_send_initial {

                    connection.send_random_request(
                        config,
                        &state,
                        &mut rng,
                    );

                    initial_sent = true;
                }
            }
        }

        // Slowly create new connections
        if token_counter < config.num_connections && iter_counter % create_conn_interval == 0 {
            let res = Connection::create_and_register(
                config,
                &mut connections,
                &mut poll,
                &mut token_counter,
            );

            if let Err(err) = res {
                eprintln!("create connection error: {}", err);
            }

            // initial_sent = false;
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}
