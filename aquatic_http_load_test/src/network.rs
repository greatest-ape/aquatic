use std::sync::atomic::Ordering;
use std::time::Duration;
use std::io::{Read, Write, ErrorKind, Cursor};

use mio::{net::TcpStream, Events, Poll, Interest, Token};
use rand::{rngs::SmallRng, prelude::*};
use slab::Slab;

use crate::common::*;
use crate::config::*;
use crate::utils::create_random_request;


pub struct Connection {
    stream: TcpStream,
    read_buffer: [u8; 4096],
    bytes_read: usize,
    can_send_initial: bool,
}


impl Connection {
    pub fn create_and_register(
        config: &Config,
        connections: &mut ConnectionMap,
        poll: &mut Poll,
        token_counter: &mut usize,
    ) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect(config.server_address)?;

        let entry = connections.vacant_entry();

        *token_counter = entry.key();
    
        poll.registry()
            .register(&mut stream, Token(*token_counter), Interest::READABLE)
            .unwrap();
        
        let connection  = Connection {
            stream,
            read_buffer: [0; 4096],
            bytes_read: 0,
            can_send_initial: true,
        };
    
        entry.insert(connection);
    
        Ok(())
    }

    pub fn read_response_and_send_request(
        &mut self,
        config: &Config,
        state: &LoadTestState,
        rng: &mut impl Rng,
        request_buffer: &mut Cursor<&mut [u8]>,
    ){
        loop {
            match self.stream.read(&mut self.read_buffer[self.bytes_read..]){
                Ok(bytes_read) => {
                    self.bytes_read += bytes_read;
                },
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    self.can_send_initial = false;

                    break;
                },
                Err(err) => {
                    self.bytes_read = 0;

                    eprintln!("handle_read_event error: {}", err);

                    return;
                }
            }
        };

        state.statistics.bytes_received
            .fetch_add(self.bytes_read, Ordering::SeqCst);

        let interesting_bytes = &self.read_buffer[..self.bytes_read];

        self.bytes_read = 0;

        Self::register_response_type(state, interesting_bytes);

        self.send_request(
            config,
            state,
            rng,
            request_buffer,
        );
    }

    /// Ultra-crappy byte searches to determine response type with some degree
    /// of certainty.
    fn register_response_type(
        state: &LoadTestState,
        response_bytes: &[u8],
    ){
        for chunk in response_bytes.windows(12){
            if chunk == b"e8:intervali" {
                state.statistics.responses_announce.fetch_add(1, Ordering::SeqCst);

                return;
            }
        }
        for chunk in response_bytes.windows(9){
            if chunk == b"d5:filesd" {
                state.statistics.responses_scrape.fetch_add(1, Ordering::SeqCst);

                return;
            }
        }
        for chunk in response_bytes.windows(18){
            if chunk == b"d14:failure_reason" {
                state.statistics.responses_failure.fetch_add(1, Ordering::SeqCst);

                return;
            }
        }
    }

    pub fn send_request(
        &mut self,
        config: &Config,
        state: &LoadTestState,
        rng: &mut impl Rng,
        request_buffer: &mut Cursor<&mut [u8]>,
    ){
        let request = create_random_request(
            &config,
            &state,
            rng
        );

        request_buffer.set_position(0);
        request.write(request_buffer).unwrap();
        let position = request_buffer.position() as usize;

        match self.send_request_inner(state, &request_buffer.get_mut()[..position]){
            Ok(_) => {
                state.statistics.requests.fetch_add(1, Ordering::SeqCst);
            },
            Err(err) => {
                eprintln!("send request error: {}", err);
            }
        }

        self.can_send_initial = false;
    }

    fn send_request_inner(
        &mut self,
        state: &LoadTestState,
        request: &[u8]
    ) -> ::std::io::Result<()> {
        let bytes_sent = self.stream.write(request)?;

        state.statistics.bytes_sent
            .fetch_add(bytes_sent, Ordering::SeqCst);

        self.stream.flush()?;

        Ok(())
    }

}


pub type ConnectionMap = Slab<Connection>;


pub fn run_socket_thread(
    config: &Config,
    state: LoadTestState,
    num_initial_requests: usize,
) {
    let timeout = Duration::from_micros(config.network.poll_timeout_microseconds);
    let create_conn_interval = 2 ^ config.network.connection_creation_interval;

    let mut connections: ConnectionMap = Slab::with_capacity(config.num_connections);
    let mut poll = Poll::new().expect("create poll");
    let mut events = Events::with_capacity(config.network.poll_event_capacity);
    let mut rng = SmallRng::from_entropy();
    let mut request_buffer = [0u8; 1024];
    let mut request_buffer = Cursor::new(&mut request_buffer[..]);

    let mut token_counter = 0usize;

    for _ in 0..num_initial_requests {
        Connection::create_and_register(
            config,
            &mut connections,
            &mut poll,
            &mut token_counter,
        ).unwrap();
    }

    let mut initial_sent = false;
    let mut iter_counter = 0usize;

    loop {
        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter(){
            if event.is_readable(){
                let token = event.token();

                if let Some(connection) = connections.get_mut(token.0){
                    connection.read_response_and_send_request(
                        config,
                        &state,
                        &mut rng,
                        &mut request_buffer
                    );
                } else {
                    eprintln!("connection not found: {:?}", token);
                }
            }
        }

        if !initial_sent {
            for (_, connection) in connections.iter_mut(){
                if connection.can_send_initial {
                    connection.send_request(
                        config,
                        &state,
                        &mut rng,
                        &mut request_buffer
                    );

                    initial_sent = true;
                }
            }
        }

        // Slowly create new connections
        if token_counter < config.num_connections && iter_counter % create_conn_interval == 0 {
            Connection::create_and_register(
                config,
                &mut connections,
                &mut poll,
                &mut token_counter,
            ).unwrap();

            initial_sent = false;
        }

        iter_counter = iter_counter.wrapping_add(1);
    }
}
