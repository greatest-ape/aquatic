use std::{cell::RefCell, convert::TryInto, io::{Cursor, ErrorKind, Read}, rc::Rc, sync::{Arc, atomic::Ordering}, time::Duration};

use aquatic_http_protocol::response::Response;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use glommio::{prelude::*, timer::TimerActionRepeat};
use glommio::net::TcpStream;
use rand::{SeedableRng, prelude::SmallRng};
use rustls::ClientConnection;

use crate::{common::LoadTestState, config::Config, utils::create_random_request};

pub async fn run_socket_thread(
    config: Config,
    tls_config: Arc<rustls::ClientConfig>,
    load_test_state: LoadTestState,
) -> anyhow::Result<()> {
    let config = Rc::new(config);
    let num_active_connections = Rc::new(RefCell::new(0usize));

    TimerActionRepeat::repeat(move || periodically_open_connections(
        config.clone(),
        tls_config.clone(),
        load_test_state.clone(),
        num_active_connections.clone())
    );

    futures_lite::future::pending::<bool>().await;

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
            if let Err(err) = Connection::run(config, tls_config, load_test_state, num_active_connections).await {
                eprintln!("connection creation error: {:?}", err);
            }
        }).detach();
    }

    Some(Duration::from_secs(1))
}

struct Connection {
    config: Rc<Config>,
    load_test_state: LoadTestState,
    rng: SmallRng,
    stream: TcpStream,
    tls: ClientConnection,
    response_buffer: [u8; 2048],
    response_buffer_position: usize,
    send_new_request: bool,
    queued_responses: usize,
}

impl Connection {
    async fn run(
        config: Rc<Config>,
        tls_config: Arc<rustls::ClientConfig>,
        load_test_state: LoadTestState,
        num_active_connections: Rc<RefCell<usize>>,
    ) -> anyhow::Result<()> {
        let stream = TcpStream::connect(config.server_address).await
            .map_err(|err| anyhow::anyhow!("connect: {:?}", err))?;
        let tls = ClientConnection::new(tls_config, "example.com".try_into().unwrap()).unwrap();
        let rng = SmallRng::from_entropy();

        let mut connection = Connection {
            config,
            load_test_state,
            rng,
            stream,
            tls,
            response_buffer: [0; 2048],
            response_buffer_position: 0,
            send_new_request: true,
            queued_responses: 0,
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
            if self.send_new_request {
                let request = create_random_request(&self.config, &self.load_test_state, &mut self.rng);

                request.write(&mut self.tls.writer())?;
                self.queued_responses += 1;

                self.send_new_request = false;
            }

            self.write_tls().await?;
            self.read_tls().await?;
        }
    }

    async fn read_tls(&mut self) -> anyhow::Result<()> {
        loop {
            let mut buf = [0u8; 1024];

            let bytes_read = self.stream.read(&mut buf).await?;

            if bytes_read == 0 {
                return Err(anyhow::anyhow!("Peer has closed connection"));
            }

            self
                .load_test_state
                .statistics
                .bytes_received
                .fetch_add(bytes_read, Ordering::SeqCst);

            let _ = self.tls.read_tls(&mut &buf[..bytes_read]).unwrap();

            let io_state = self.tls.process_new_packets()?;

            let mut added_plaintext = false;

            if io_state.plaintext_bytes_to_read() != 0 {
                loop {
                    match self.tls.reader().read(&mut buf) {
                        Ok(0) => {
                            break;
                        }
                        Ok(amt) => {
                            let end = self.response_buffer_position + amt;

                            if end > self.response_buffer.len() {
                                return Err(anyhow::anyhow!("response too large"));
                            } else {
                                let response_buffer_slice = &mut self.response_buffer[self.response_buffer_position..end];

                                response_buffer_slice.copy_from_slice(&buf[..amt]);

                                self.response_buffer_position = end;

                                added_plaintext = true;
                            }
                        }
                        Err(err) if err.kind() == ErrorKind::WouldBlock => {
                            break;
                        }
                        Err(err) => {
                            panic!("tls.reader().read: {}", err);
                        }
                    }
                }
            }

            if added_plaintext {
                let interesting_bytes = &self.response_buffer[..self.response_buffer_position];

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

                            match response {
                                Response::Announce(_) => {
                                    self
                                        .load_test_state
                                        .statistics
                                        .responses_announce
                                        .fetch_add(1, Ordering::SeqCst);
                                }
                                Response::Scrape(_) => {
                                    self
                                        .load_test_state
                                        .statistics
                                        .responses_scrape
                                        .fetch_add(1, Ordering::SeqCst);
                                }
                                Response::Failure(response) => {
                                    self
                                        .load_test_state
                                        .statistics
                                        .responses_failure
                                        .fetch_add(1, Ordering::SeqCst);
                                    println!(
                                        "failure response: reason: {}",
                                        response.failure_reason
                                    );
                                }
                            }

                            self.response_buffer_position = 0;
                            self.send_new_request = true;

                            break;
                        }
                        Err(err) => {
                            eprintln!(
                                "deserialize response error with {} bytes read: {:?}, text: {}",
                                self.response_buffer_position,
                                err,
                                String::from_utf8_lossy(interesting_bytes)
                            );
                        }
                    }
                }
            }

            if self.tls.wants_write() {
                break;
            }
        }

        Ok(())
    }

    async fn write_tls(&mut self) -> anyhow::Result<()> {
        if !self.tls.wants_write() {
            return Ok(());
        }

        let mut buf = Vec::new();
        let mut buf = Cursor::new(&mut buf);

        while self.tls.wants_write() {
            self.tls.write_tls(&mut buf).unwrap();
        }

        let len = buf.get_ref().len();

        self.stream.write_all(&buf.into_inner()).await?;
        self.stream.flush().await?;

        self
            .load_test_state
            .statistics
            .bytes_sent
            .fetch_add(len, Ordering::SeqCst);

        if self.queued_responses != 0 {
            self.load_test_state.statistics.requests.fetch_add(self.queued_responses, Ordering::SeqCst);

            self.queued_responses = 0;
        }

        Ok(())
    }
}