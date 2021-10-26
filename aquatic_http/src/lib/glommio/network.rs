use std::cell::RefCell;
use std::io::{BufReader, Cursor, ErrorKind, Read, Write};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aquatic_http_protocol::request::{Request, RequestParseError};
use aquatic_http_protocol::response::{FailureResponse, Response};
use futures_lite::{AsyncReadExt, AsyncWriteExt, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Receivers, Role, Senders};
use glommio::channels::shared_channel::{ConnectedReceiver, ConnectedSender, SharedSender};
use glommio::prelude::*;
use glommio::net::{TcpListener, TcpStream};
use glommio::channels::local_channel::{new_bounded, LocalReceiver, LocalSender};
use glommio::task::JoinHandle;
use rustls::{IoState, ServerConnection};
use slab::Slab;

use crate::common::num_digits_in_usize;
use crate::config::Config;

const BUFFER_SIZE: usize = 1024;

#[derive(Clone, Copy, Debug)]
pub struct ConnectionId(pub usize);

struct ConnectionReference {
    response_sender: LocalSender<Response>,
    handle: JoinHandle<()>,
}

struct Connection {
    // request_senders: Rc<Senders<(ConnectionId, Request)>>,
    response_receiver: LocalReceiver<Response>,
    tls: ServerConnection,
    stream: TcpStream,
    index: ConnectionId,
    request_buffer: Vec<u8>,
    wait_for_response: bool,
    close_after_writing: bool,
}

pub async fn run_socket_worker(
    config: Config,
    tls_config: Arc<rustls::ServerConfig>,
    request_mesh_builder: MeshBuilder<(ConnectionId, Request), Partial>,
    response_mesh_builder: MeshBuilder<(ConnectionId, Response), Partial>,
    num_bound_sockets: Arc<AtomicUsize>,
) {
    let config = Rc::new(config);

    let listener = TcpListener::bind(config.network.address).expect("bind socket");
    num_bound_sockets.fetch_add(1, Ordering::SeqCst);

    // let (_, mut response_receivers) = response_mesh_builder.join(Role::Consumer).await.unwrap();

    // let (request_senders, _) = request_mesh_builder.join(Role::Producer).await.unwrap();
    // let request_senders = Rc::new(request_senders);

    let connection_slab = Rc::new(RefCell::new(Slab::new()));

    // for (_, response_receiver) in response_receivers.streams() {
    //     spawn_local(receive_responses(response_receiver, connection_slab.clone())).detach();
    // }

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let (response_sender, response_receiver) = new_bounded(1);

                let mut slab = connection_slab.borrow_mut();
                let entry = slab.vacant_entry();

                let conn = Connection {
                    // request_senders: request_senders.clone(),
                    response_receiver,
                    tls: ServerConnection::new(tls_config.clone()).unwrap(),
                    stream,
                    index: ConnectionId(entry.key()),
                    request_buffer: Vec::new(),
                    wait_for_response: false,
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
    mut response_receiver: ConnectedReceiver<(ConnectionId, Response)>,
    connection_references: Rc<RefCell<Slab<ConnectionReference>>>,
) {
    while let Some((connection_id, response)) = response_receiver.next().await {
        if let Some(reference) = connection_references.borrow().get(connection_id.0) {
            reference.response_sender.try_send(response);
        }
    }
}

impl Connection {
    async fn handle_stream(&mut self) -> anyhow::Result<()> {
        loop {
            self.read_tls().await?;

            if self.wait_for_response {
                if let Some(response) = self.response_receiver.recv().await {
                    self.queue_response(&response)?;

                    self.wait_for_response = false;

                    // TODO: trigger close here if keepalive is false
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

    async fn read_tls(&mut self) -> anyhow::Result<()> {
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
                        self.wait_for_response = true;

                        ::log::trace!("received request: {:?}", request);
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

        Ok(())
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
