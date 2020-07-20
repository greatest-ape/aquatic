use std::sync::atomic::Ordering;
use std::time::Duration;
use std::io::{Read, Write, ErrorKind};

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
    ){
        loop {
            match self.stream.read(&mut self.read_buffer[self.bytes_read..]){
                Ok(bytes_read) => {
                    self.bytes_read = bytes_read;

                    break;
                },
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    self.can_send_initial = false;

                    eprintln!("handle_read_event error would block: {}", err);

                    return;
                },
                Err(err) => {
                    self.bytes_read = 0;

                    eprintln!("handle_read_event error: {}", err);

                    return;
                }
            }
        };

        let res_response = Response::from_bytes(
            &self.read_buffer[..self.bytes_read]
        );

        self.bytes_read = 0;

        match res_response {
            Ok(Response::Announce(_)) => {
                state.statistics.responses_announce
                    .fetch_add(1, Ordering::SeqCst);
            },
            Ok(Response::Scrape(_)) => {
                state.statistics.responses_scrape
                    .fetch_add(1, Ordering::SeqCst);
            },
            Ok(Response::Failure(_)) => {
                state.statistics.responses_failure
                    .fetch_add(1, Ordering::SeqCst);
            },
            Err(err) => {
                eprintln!("response from bytes error: {}", err);

                return;
            }
        }

        self.send_request(
            config,
            state,
            rng
        );
    }

    pub fn send_request(
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

        match self.send_request_inner(&request.as_bytes()){
            Ok(_) => {
                state.statistics.requests.fetch_add(1, Ordering::SeqCst);
            },
            Err(err) => {
                eprintln!("send request error: {}", err);
            }
        }

        self.can_send_initial = false;
    }

    fn send_request_inner(&mut self, request: &[u8]) -> ::std::io::Result<()> {
        self.stream.write(request)?;
        self.stream.flush()?;

        Ok(())
    }

}


pub type ConnectionMap = Slab<Connection>;


const NUM_CONNECTIONS: usize = 128;


pub fn run_socket_thread(
    config: &Config,
    state: LoadTestState,
    num_initial_requests: usize,
) {
    let timeout = Duration::from_micros(config.network.poll_timeout_microseconds);

    let mut connections: ConnectionMap = Slab::with_capacity(NUM_CONNECTIONS);
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

    let mut initial_sent = false;
    let mut iter_counter = 0usize;

    const CREATE_CONN_INTERVAL: usize = 2 ^ 16;

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
                        &mut rng
                    );
                } else {
                    eprintln!("connection not found: {:?}", token);
                }
            }
        }

        if !initial_sent {
            for (_, connection) in connections.iter_mut(){
                if connection.can_send_initial {
                    connection.send_request(config, &state, &mut rng);

                    initial_sent = true;
                }
            }
        }

        // Slowly create new connections
        if token_counter < NUM_CONNECTIONS && iter_counter % CREATE_CONN_INTERVAL == 0 {
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
