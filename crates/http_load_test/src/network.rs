use std::{
    cell::RefCell,
    convert::TryInto,
    io::Cursor,
    rc::Rc,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use aquatic_http_protocol::response::Response;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use futures_rustls::TlsConnector;
use glommio::net::TcpStream;
use glommio::{prelude::*, timer::TimerActionRepeat};
use rand::{prelude::SmallRng, SeedableRng};

use crate::{common::LoadTestState, config::Config, utils::create_random_request};

pub async fn run_socket_thread(
    config: Config,
    opt_tls_config: Option<Arc<rustls::ClientConfig>>,
    load_test_state: LoadTestState,
) -> anyhow::Result<()> {
    let config = Rc::new(config);
    let num_active_connections = Rc::new(RefCell::new(0usize));
    let rng = Rc::new(RefCell::new(SmallRng::from_entropy()));

    let interval = config.connection_creation_interval_ms;

    if interval == 0 {
        loop {
            if *num_active_connections.borrow() < config.num_connections {
                if let Err(err) = run_connection(
                    config.clone(),
                    opt_tls_config.clone(),
                    load_test_state.clone(),
                    num_active_connections.clone(),
                    rng.clone(),
                )
                .await
                {
                    ::log::error!("connection creation error: {:?}", err);
                }
            }
        }
    } else {
        let interval = Duration::from_millis(interval);

        TimerActionRepeat::repeat(move || {
            periodically_open_connections(
                config.clone(),
                interval,
                opt_tls_config.clone(),
                load_test_state.clone(),
                num_active_connections.clone(),
                rng.clone(),
            )
        });
    }

    futures_lite::future::pending::<bool>().await;

    Ok(())
}

async fn periodically_open_connections(
    config: Rc<Config>,
    interval: Duration,
    opt_tls_config: Option<Arc<rustls::ClientConfig>>,
    load_test_state: LoadTestState,
    num_active_connections: Rc<RefCell<usize>>,
    rng: Rc<RefCell<SmallRng>>,
) -> Option<Duration> {
    if *num_active_connections.borrow() < config.num_connections {
        spawn_local(async move {
            if let Err(err) = run_connection(
                config,
                opt_tls_config,
                load_test_state,
                num_active_connections,
                rng.clone(),
            )
            .await
            {
                ::log::error!("connection creation error: {:?}", err);
            }
        })
        .detach();
    }

    Some(interval)
}

async fn run_connection(
    config: Rc<Config>,
    opt_tls_config: Option<Arc<rustls::ClientConfig>>,
    load_test_state: LoadTestState,
    num_active_connections: Rc<RefCell<usize>>,
    rng: Rc<RefCell<SmallRng>>,
) -> anyhow::Result<()> {
    let stream = TcpStream::connect(config.server_address)
        .await
        .map_err(|err| anyhow::anyhow!("connect: {:?}", err))?;

    if let Some(tls_config) = opt_tls_config {
        let stream = TlsConnector::from(tls_config)
            .connect("example.com".try_into().unwrap(), stream)
            .await?;

        let mut connection = Connection {
            config,
            load_test_state,
            rng,
            stream,
            buffer: Box::new([0; 2048]),
        };

        connection.run(num_active_connections).await?;
    } else {
        let mut connection = Connection {
            config,
            load_test_state,
            rng,
            stream,
            buffer: Box::new([0; 2048]),
        };

        connection.run(num_active_connections).await?;
    }

    Ok(())
}

struct Connection<S> {
    config: Rc<Config>,
    load_test_state: LoadTestState,
    rng: Rc<RefCell<SmallRng>>,
    stream: S,
    buffer: Box<[u8; 2048]>,
}

impl<S> Connection<S>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin + 'static,
{
    async fn run(&mut self, num_active_connections: Rc<RefCell<usize>>) -> anyhow::Result<()> {
        *num_active_connections.borrow_mut() += 1;

        let result = self.run_connection_loop().await;

        if let Err(err) = &result {
            ::log::info!("connection error: {:?}", err);
        }

        *num_active_connections.borrow_mut() -= 1;

        result
    }

    async fn run_connection_loop(&mut self) -> anyhow::Result<()> {
        loop {
            self.send_request().await?;
            self.read_response().await?;

            if !self.config.keep_alive {
                break Ok(());
            }
        }
    }

    async fn send_request(&mut self) -> anyhow::Result<()> {
        let request = create_random_request(
            &self.config,
            &self.load_test_state,
            &mut self.rng.borrow_mut(),
        );

        let mut cursor = Cursor::new(&mut self.buffer[..]);

        request.write(&mut cursor, self.config.url_suffix.as_bytes())?;

        let cursor_position = cursor.position() as usize;

        let bytes_sent = self
            .stream
            .write(&cursor.into_inner()[..cursor_position])
            .await?;

        self.stream.flush().await?;

        self.load_test_state
            .statistics
            .bytes_sent
            .fetch_add(bytes_sent, Ordering::Relaxed);

        self.load_test_state
            .statistics
            .requests
            .fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    async fn read_response(&mut self) -> anyhow::Result<()> {
        let mut buffer_position = 0;

        loop {
            let bytes_read = self
                .stream
                .read(&mut self.buffer[buffer_position..])
                .await?;

            if bytes_read == 0 {
                break;
            }

            buffer_position += bytes_read;

            let interesting_bytes = &self.buffer[..buffer_position];

            let mut opt_body_start_index = None;

            for (i, chunk) in interesting_bytes.windows(4).enumerate() {
                if chunk == b"\r\n\r\n" {
                    opt_body_start_index = Some(i + 4);

                    break;
                }
            }

            if let Some(body_start_index) = opt_body_start_index {
                match Response::parse_bytes(&interesting_bytes[body_start_index..]) {
                    Ok(response) => {
                        match response {
                            Response::Announce(_) => {
                                self.load_test_state
                                    .statistics
                                    .responses_announce
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                            Response::Scrape(_) => {
                                self.load_test_state
                                    .statistics
                                    .responses_scrape
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                            Response::Failure(response) => {
                                self.load_test_state
                                    .statistics
                                    .responses_failure
                                    .fetch_add(1, Ordering::Relaxed);
                                println!("failure response: reason: {}", response.failure_reason);
                            }
                        }

                        self.load_test_state
                            .statistics
                            .bytes_received
                            .fetch_add(interesting_bytes.len(), Ordering::Relaxed);

                        break;
                    }
                    Err(err) => {
                        ::log::warn!(
                            "deserialize response error with {} bytes read: {:?}, text: {}",
                            buffer_position,
                            err,
                            interesting_bytes.escape_ascii()
                        );
                    }
                }
            }
        }

        Ok(())
    }
}
