use std::sync::atomic::Ordering;
use std::time::Duration;
use std::io::{Read, Write, ErrorKind};

use hashbrown::HashMap;
use mio::{net::TcpStream, Events, Poll, Interest, Token};
use rand::{rngs::SmallRng, prelude::*};

use crate::common::*;
use crate::config::*;
use crate::utils::create_random_request;


pub struct Connection {
    stream: TcpStream,
    read_buffer: [u8; 2048],
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
        
        let connection  = Connection {
            stream,
            read_buffer: [0; 2048],
            bytes_read: 0,
            can_send: true,
        };
    
        connections.insert(*token_counter, connection);

        *token_counter = token_counter.wrapping_add(1);
    
        Ok(())
    }

    pub fn read_response_and_send_request(
        &mut self,
        config: &Config,
        state: &LoadTestState,
        rng: &mut impl Rng,
    ) -> bool { // true = response received
        loop {
            match self.stream.read(&mut self.read_buffer){
                Ok(bytes_read) => {
                    self.bytes_read = bytes_read;

                    break;
                },
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    self.can_send = false;

                    eprintln!("handle_read_event error would block: {}", err);

                    return false;
                },
                Err(err) => {
                    self.bytes_read = 0;

                    eprintln!("handle_read_event error: {}", err);

                    return false;
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

                return false;
            }
        }

        self.send_request(
            config,
            state,
            rng
        );

        true
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

        self.can_send = false;
    }

    fn send_request_inner(&mut self, request: &[u8]) -> ::std::io::Result<()> {
        self.stream.write(request)?;
        self.stream.flush()?;

        Ok(())
    }

}


pub type ConnectionMap = HashMap<usize, Connection>;


pub fn run_socket_thread(
    config: &Config,
    state: LoadTestState,
    num_initial_requests: usize,
) {
    let timeout = Duration::from_micros(config.network.poll_timeout_microseconds);

    let mut connections: ConnectionMap = HashMap::new();
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

    loop {
        let mut responses_received = 0usize;

        poll.poll(&mut events, Some(timeout))
            .expect("failed polling");

        for event in events.iter(){
            if event.is_readable(){
                let token = event.token();

                if let Some(connection) = connections.get_mut(&token.0){
                    if connection.read_response_and_send_request(config, &state, &mut rng){
                        responses_received += 1;
                    }
                } else {
                    eprintln!("connection not found: {:?}", token);
                }
            }
        }

        for (_, connection) in connections.iter_mut(){
            if connection.can_send {
                connection.send_request(config, &state, &mut rng);
            }
        }

        if token_counter < 1 && responses_received > 0 {
            Connection::create_and_register(
                config,
                &mut connections,
                &mut poll,
                &mut token_counter,
            ).unwrap();
        }
    }
}
