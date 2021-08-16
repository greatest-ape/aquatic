use std::io::{Cursor, ErrorKind, Read, Write};
use std::sync::atomic::Ordering;
use std::time::Duration;

use hashbrown::HashMap;
use mio::{net::TcpStream, Events, Interest, Poll, Token};
use rand::{prelude::*, rngs::SmallRng};

use crate::common::*;
use crate::config::*;
use crate::utils::create_random_request;

pub struct Connection {
    stream: TcpStream,
    read_buffer: [u8; 4096],
    bytes_read: usize,
    can_send: bool,
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

        let connection = Connection {
            stream,
            read_buffer: [0; 4096],
            bytes_read: 0,
            can_send: true,
        };

        connections.insert(*token_counter, connection);

        *token_counter = token_counter.wrapping_add(1);

        Ok(())
    }

    pub fn read_response(&mut self, state: &LoadTestState) -> bool {
        // bool = remove connection
        loop {
            match self.stream.read(&mut self.read_buffer[self.bytes_read..]) {
                Ok(0) => {
                    if self.bytes_read == self.read_buffer.len() {
                        eprintln!("read buffer is full");
                    }

                    break true;
                }
                Ok(bytes_read) => {
                    self.bytes_read += bytes_read;

                    let interesting_bytes = &self.read_buffer[..self.bytes_read];

                    let mut opt_body_start_index = None;

                    for (i, chunk) in interesting_bytes.windows(4).enumerate() {
                        if chunk == b"\r\n\r\n" {
                            opt_body_start_index = Some(i + 4);

                            break;
                        }
                    }

                    if let Some(body_start_index) = opt_body_start_index {
                        let interesting_bytes = &interesting_bytes[body_start_index..];

                        match Response::from_bytes(interesting_bytes) {
                            Ok(response) => {
                                state
                                    .statistics
                                    .bytes_received
                                    .fetch_add(self.bytes_read, Ordering::SeqCst);

                                match response {
                                    Response::Announce(_) => {
                                        state
                                            .statistics
                                            .responses_announce
                                            .fetch_add(1, Ordering::SeqCst);
                                    }
                                    Response::Scrape(_) => {
                                        state
                                            .statistics
                                            .responses_scrape
                                            .fetch_add(1, Ordering::SeqCst);
                                    }
                                    Response::Failure(response) => {
                                        state
                                            .statistics
                                            .responses_failure
                                            .fetch_add(1, Ordering::SeqCst);
                                        println!(
                                            "failure response: reason: {}",
                                            response.failure_reason
                                        );
                                    }
                                }

                                self.bytes_read = 0;
                                self.can_send = true;
                            }
                            Err(err) => {
                                eprintln!(
                                    "deserialize response error with {} bytes read: {:?}, text: {}",
                                    self.bytes_read,
                                    err,
                                    String::from_utf8_lossy(interesting_bytes)
                                );
                            }
                        }
                    }
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    break false;
                }
                Err(_) => {
                    self.bytes_read = 0;

                    break true;
                }
            }
        }
    }

    pub fn send_request(
        &mut self,
        config: &Config,
        state: &LoadTestState,
        rng: &mut impl Rng,
        request_buffer: &mut Cursor<&mut [u8]>,
    ) -> bool {
        // bool = remove connection
        if !self.can_send {
            return false;
        }

        let request = create_random_request(&config, &state, rng);

        request_buffer.set_position(0);
        request.write(request_buffer).unwrap();
        let position = request_buffer.position() as usize;

        match self.send_request_inner(state, &request_buffer.get_mut()[..position]) {
            Ok(()) => {
                state.statistics.requests.fetch_add(1, Ordering::SeqCst);

                self.can_send = false;

                false
            }
            Err(_) => true,
        }
    }

    fn send_request_inner(
        &mut self,
        state: &LoadTestState,
        request: &[u8],
    ) -> ::std::io::Result<()> {
        let bytes_sent = self.stream.write(request)?;

        state
            .statistics
            .bytes_sent
            .fetch_add(bytes_sent, Ordering::SeqCst);

        self.stream.flush()?;

        Ok(())
    }

    fn deregister(&mut self, poll: &mut Poll) -> ::std::io::Result<()> {
        poll.registry().deregister(&mut self.stream)
    }
}

pub type ConnectionMap = HashMap<usize, Connection>;

pub fn run_socket_thread(config: &Config, state: LoadTestState, num_initial_requests: usize) {
    let timeout = Duration::from_micros(config.network.poll_timeout_microseconds);
    let create_conn_interval = 2 ^ config.network.connection_creation_interval;

    let mut connections: ConnectionMap = HashMap::with_capacity(config.num_connections);
    let mut poll = Poll::new().expect("create poll");
    let mut events = Events::with_capacity(config.network.poll_event_capacity);
    let mut rng = SmallRng::from_entropy();
    let mut request_buffer = [0u8; 1024];
    let mut request_buffer = Cursor::new(&mut request_buffer[..]);

    let mut token_counter = 0usize;

    for _ in 0..num_initial_requests {
        Connection::create_and_register(config, &mut connections, &mut poll, &mut token_counter)
            .unwrap();
    }

    let mut iter_counter = 0usize;
    let mut num_to_create = 0usize;

    let mut drop_connections = Vec::with_capacity(config.num_connections);

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter() {
            if event.is_readable() {
                let token = event.token();

                if let Some(connection) = connections.get_mut(&token.0) {
                    // Note that this does not indicate successfully reading
                    // response
                    if connection.read_response(&state) {
                        remove_connection(&mut poll, &mut connections, token.0);

                        num_to_create += 1;
                    }
                } else {
                    eprintln!("connection not found: {:?}", token);
                }
            }
        }

        for (k, connection) in connections.iter_mut() {
            let remove_connection =
                connection.send_request(config, &state, &mut rng, &mut request_buffer);

            if remove_connection {
                drop_connections.push(*k);
            }
        }

        for k in drop_connections.drain(..) {
            remove_connection(&mut poll, &mut connections, k);

            num_to_create += 1;
        }

        let max_new = config.num_connections - connections.len();

        if iter_counter % create_conn_interval == 0 {
            num_to_create += 1;
        }

        num_to_create = num_to_create.min(max_new);

        for _ in 0..num_to_create {
            let ok = Connection::create_and_register(
                config,
                &mut connections,
                &mut poll,
                &mut token_counter,
            )
            .is_ok();

            if ok {
                num_to_create -= 1;
            }
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}

fn remove_connection(poll: &mut Poll, connections: &mut ConnectionMap, connection_id: usize) {
    if let Some(mut connection) = connections.remove(&connection_id) {
        if let Err(err) = connection.deregister(poll) {
            eprintln!("couldn't deregister connection: {}", err);
        }
    }
}
