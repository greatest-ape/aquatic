use std::cell::RefCell;
use std::io::{BufReader, Cursor, ErrorKind, Read};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aquatic_http_protocol::request::{Request, RequestParseError};
use aquatic_http_protocol::response::Response;
use futures_lite::{AsyncReadExt, AsyncWriteExt, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Receivers, Role, Senders};
use glommio::channels::shared_channel::{ConnectedReceiver, ConnectedSender, SharedSender};
use glommio::prelude::*;
use glommio::net::{TcpListener, TcpStream};
use glommio::channels::local_channel::{new_bounded, LocalReceiver, LocalSender};
use glommio::task::JoinHandle;
use rustls::{IoState, ServerConnection};
use slab::Slab;

use crate::config::Config;

#[derive(Clone, Copy, Debug)]
pub struct ConnectionId(pub usize);

struct ConnectionReference {
    response_sender: LocalSender<Response>,
    handle: JoinHandle<()>,
}

struct Connection {
    // request_senders: Rc<Senders<(ConnectionId, Request)>>,
    // response_receiver: LocalReceiver<Response>,
    tls: ServerConnection,
    stream: TcpStream,
    index: ConnectionId,
    expects_request: bool,
    request_buffer: Vec<u8>,
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
                    // response_receiver,
                    tls: ServerConnection::new(tls_config.clone()).unwrap(),
                    stream,
                    index: ConnectionId(entry.key()),
                    expects_request: true,
                    request_buffer: Vec::new(),
                };

                async fn handle_stream(mut conn: Connection) {
                    if let Err(err) = conn.handle_stream().await {
                        ::log::error!("conn.handle_stream() error: {:?}", err);
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
        ::log::info!("incoming stream");
        loop {
            self.write_tls().await?;
            self.read_tls().await?;

            /*
            if !self.tls.is_handshaking() {
                if self.expects_request {
                    let request = self.extract_request()?;

                    ::log::info!("request received: {:?}", request);

                    // self.request_senders.try_send_to(0, (self.index, request));
                    self.expects_request = false;

                }/*
                else if let Some(response) = self.response_receiver.recv().await {
                    response.write(&mut self.tls.writer())?;

                    self.expects_request = true;
                } */
            }
            */
        }
    }

    async fn read_tls(&mut self) -> anyhow::Result<()> {
        loop {
            ::log::info!("read_tls (wants read)");

            let mut buf = [0u8; 1024];

            let bytes_read = self.stream.read(&mut buf).await?;

            if bytes_read == 0 {
                // Peer has closed connection. Remove it.
                return Err(anyhow::anyhow!("peer has closed connection"));
            }

            let _ = self.tls.read_tls(&mut &buf[..bytes_read]).unwrap();

            let io_state = self.tls.process_new_packets()?;

            let mut added_plaintext = false;

            while io_state.plaintext_bytes_to_read() != 0 {
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
                        ::log::info!("tls.reader().read error: {:?}", err);

                        break;
                    }
                }
            }

            if added_plaintext {
                match Request::from_bytes(&self.request_buffer[..]) {
                    Ok(request) => {
                        self.expects_request = false;

                        ::log::info!("received request: {:?}", request);
                    }
                    Err(RequestParseError::NeedMoreData) => {
                        ::log::info!("need more request data. current data: {:?}", std::str::from_utf8(&self.request_buffer));
                    }
                    Err(RequestParseError::Invalid(err)) => {
                        return Err(anyhow::anyhow!("request parse error: {:?}", err));
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

        ::log::info!("write_tls (wants write)");

        let mut buf = Vec::new();
        let mut buf = Cursor::new(&mut buf);

        while self.tls.wants_write() {
            self.tls.write_tls(&mut buf).unwrap();
        }

        self.stream.write_all(&buf.into_inner()).await.unwrap();

        Ok(())
    }
}
