use std::cell::RefCell;
use std::io::{Cursor, ErrorKind, Read, Write};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aquatic_http_protocol::common::InfoHash;
use aquatic_http_protocol::request::{AnnounceRequest, Request, RequestParseError};
use aquatic_http_protocol::response::{FailureResponse, Response};
use futures_lite::{AsyncReadExt, AsyncWriteExt, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Receivers, Role, Senders};
use glommio::channels::shared_channel::{ConnectedReceiver, ConnectedSender, SharedSender};
use glommio::prelude::*;
use glommio::net::{TcpListener, TcpStream};
use glommio::channels::local_channel::{new_bounded, LocalReceiver, LocalSender};
use glommio::task::JoinHandle;
use rustls::{ServerConnection};
use slab::Slab;

use crate::common::num_digits_in_usize;
use crate::config::Config;

use super::common::*;

const BUFFER_SIZE: usize = 1024;


struct ConnectionReference {
    response_sender: LocalSender<ChannelResponse>,
    handle: JoinHandle<()>,
}

struct Connection {
    config: Rc<Config>,
    request_senders: Rc<Senders<ChannelRequest>>,
    response_receiver: LocalReceiver<ChannelResponse>,
    response_consumer_id: ConsumerId,
    tls: ServerConnection,
    stream: TcpStream,
    connection_id: ConnectionId,
    request_buffer: Vec<u8>,
    close_after_writing: bool,
}

pub async fn run_socket_worker(
    config: Config,
    tls_config: Arc<rustls::ServerConfig>,
    request_mesh_builder: MeshBuilder<ChannelRequest, Partial>,
    response_mesh_builder: MeshBuilder<ChannelResponse, Partial>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let config = Rc::new(config);

    let listener = TcpListener::bind(config.network.address).expect("bind socket");
    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    let (_, mut response_receivers) = response_mesh_builder.join(Role::Consumer).await.unwrap();

    let response_consumer_id = ConsumerId(response_receivers.consumer_id().unwrap());

    let (request_senders, _) = request_mesh_builder.join(Role::Producer).await.unwrap();
    let request_senders = Rc::new(request_senders);

    let connection_slab = Rc::new(RefCell::new(Slab::new()));

    for (_, response_receiver) in response_receivers.streams() {
        spawn_local(receive_responses(response_receiver, connection_slab.clone())).detach();
    }

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let (response_sender, response_receiver) = new_bounded(1);

                let mut slab = connection_slab.borrow_mut();
                let entry = slab.vacant_entry();

                let conn = Connection {
                    config: config.clone(),
                    request_senders: request_senders.clone(),
                    response_receiver,
                    response_consumer_id,
                    tls: ServerConnection::new(tls_config.clone()).unwrap(),
                    stream,
                    connection_id: ConnectionId(entry.key()),
                    request_buffer: Vec::new(),
                    close_after_writing: false,
                };

                async fn handle_stream(mut conn: Connection) {
                    if let Err(err) = conn.handle_stream().await {
                        ::log::info!("conn.handle_stream() error: {:?}", err);
                    }
                }

                let handle = spawn_local(handle_stream(conn)).detach();

                let connection_reference = ConnectionReference {
                    response_sender,
                    handle,
                };

                entry.insert(connection_reference);
            },
            Err(err) => {
                ::log::error!("accept connection: {:?}", err);
            }
        }
        
    }
}

async fn receive_responses(
    mut response_receiver: ConnectedReceiver<ChannelResponse>,
    connection_references: Rc<RefCell<Slab<ConnectionReference>>>,
) {
    while let Some(channel_response) = response_receiver.next().await {
        if let Some(reference) = connection_references.borrow().get(channel_response.get_connection_id().0) {
            reference.response_sender.try_send(channel_response);
        }
    }
}

impl Connection {
    async fn handle_stream(&mut self) -> anyhow::Result<()> {
        loop {
            let opt_request = self.read_tls().await?;

            if let Some(request) = opt_request {
                let peer_addr = self.stream
                    .peer_addr()
                    .map_err(|err| anyhow::anyhow!("Couldn't get peer addr: {:?}", err))?;
                
                match request {
                    Request::Announce(request@AnnounceRequest { info_hash, .. }) => {
                        let request = ChannelRequest::Announce {
                            request,
                            connection_id: self.connection_id,
                            response_consumer_id: self.response_consumer_id,
                            peer_addr,
                        };

                        let consumer_index = calculate_request_consumer_index(&self.config, info_hash);
                        self.request_senders.try_send_to(consumer_index, request);
                    },
                    Request::Scrape(request) => {
                        // TODO
                    },
                }

                // Wait for response to arrive, then send it
                if let Some(channel_response) = self.response_receiver.recv().await {
                    if channel_response.get_peer_addr() != peer_addr {
                        return Err(anyhow::anyhow!("peer addressess didn't match"));
                    }
                    
                    let opt_response = match channel_response {
                        ChannelResponse::Announce { response, ..  } => {
                            Some(Response::Announce(response))
                        }
                        ChannelResponse::Scrape { response, original_indices, .. } => {
                            None // TODO: accumulate scrape requests
                        }
                    };

                    if let Some(response) = opt_response {
                        self.queue_response(&response)?;

                        if !self.config.network.keep_alive {
                            self.close_after_writing = true;
                        }
                    }
                }
            }

            self.write_tls().await?;

            if self.close_after_writing {
                let _ = self.stream.shutdown(std::net::Shutdown::Both).await;

                break;
            }
        }

        Ok(())
    }

    async fn read_tls(&mut self) -> anyhow::Result<Option<Request>> {
        loop {
            ::log::debug!("read_tls");

            let mut buf = [0u8; BUFFER_SIZE];

            let bytes_read = self.stream.read(&mut buf).await?;

            if bytes_read == 0 {
                ::log::debug!("peer has closed connection");

                self.close_after_writing = true;

                break;
            }

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
                            self.request_buffer.extend_from_slice(&buf[..amt]);

                            added_plaintext = true;
                        },
                        Err(err) if err.kind() == ErrorKind::WouldBlock => {
                            break;
                        }
                        Err(err) => {
                            // Should never happen
                            ::log::error!("tls.reader().read error: {:?}", err);

                            break;
                        }
                    }
                }
            }

            if added_plaintext {
                match Request::from_bytes(&self.request_buffer[..]) {
                    Ok(request) => {
                        ::log::debug!("received request: {:?}", request);

                        return Ok(Some(request));
                    }
                    Err(RequestParseError::NeedMoreData) => {
                        ::log::debug!("need more request data. current data: {:?}", std::str::from_utf8(&self.request_buffer));
                    }
                    Err(RequestParseError::Invalid(err)) => {
                        ::log::debug!("invalid request: {:?}", err);

                        let response = Response::Failure(FailureResponse {
                            failure_reason: "Invalid request".into(),
                        });

                        self.queue_response(&response)?;
                        self.close_after_writing = true;

                        break;
                    }
                }
            }

            if self.tls.wants_write() {
                break
            }
        }

        Ok(None)
    }

    async fn write_tls(&mut self) -> anyhow::Result<()> {
        if !self.tls.wants_write() {
            return Ok(());
        }

        ::log::debug!("write_tls (wants write)");

        let mut buf = Vec::new();
        let mut buf = Cursor::new(&mut buf);

        while self.tls.wants_write() {
            self.tls.write_tls(&mut buf).unwrap();
        }

        self.stream.write_all(&buf.into_inner()).await?;
        self.stream.flush().await?;

        Ok(())
    }

    fn queue_response(&mut self, response: &Response) -> anyhow::Result<()> {
        let mut body = Vec::new();

        response.write(&mut body).unwrap();

        let content_len = body.len() + 2; // 2 is for newlines at end
        let content_len_num_digits = num_digits_in_usize(content_len);

        let mut response_bytes = Vec::with_capacity(39 + content_len_num_digits + body.len());

        response_bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: ");
        ::itoa::write(&mut response_bytes, content_len)?;
        response_bytes.extend_from_slice(b"\r\n\r\n");
        response_bytes.append(&mut body);
        response_bytes.extend_from_slice(b"\r\n");

        self.tls.writer().write(&response_bytes[..])?;

        Ok(())
    }
}

fn calculate_request_consumer_index(config: &Config, info_hash: InfoHash) -> usize {
    (info_hash.0[0] as usize) % config.request_workers
}