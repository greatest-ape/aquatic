use std::cell::RefCell;
use std::io::{BufReader, Cursor, Read};
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
    request_senders: Rc<Senders<(ConnectionId, Request)>>,
    response_receiver: LocalReceiver<Response>,
    tls: ServerConnection,
    stream: TcpStream,
    index: ConnectionId,
    expects_request: bool,
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

    let (_, mut response_receivers) = response_mesh_builder.join(Role::Consumer).await.unwrap();

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
                    request_senders: request_senders.clone(),
                    response_receiver,
                    tls: ServerConnection::new(tls_config.clone()).unwrap(),
                    stream,
                    index: ConnectionId(entry.key()),
                    expects_request: true,
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
        loop {
            self.write_tls().await?;
            self.read_tls().await;

            if !self.tls.is_handshaking() {
                if self.expects_request {
                    let request = self.extract_request()?;

                    self.request_senders.try_send_to(0, (self.index, request));
                    self.expects_request = false;

                } else if let Some(response) = self.response_receiver.recv().await {
                    response.write(&mut self.tls.writer())?;

                    self.expects_request = true;
                }
            }
        }
    }

    async fn read_tls(&mut self) -> anyhow::Result<()> {
        while self.tls.wants_read() {
            let mut buf = Vec::new();

            let _ciphertext_bytes_read = self.stream.read_to_end(&mut buf).await?;

            let mut cursor = Cursor::new(&buf[..]);

            let _plaintext_bytes_read = self.tls.read_tls(&mut cursor)?;

            let _io_state = self.tls.process_new_packets()?;
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
            self.tls.write_tls(&mut buf)?;
        }

        self.stream.write_all(&buf.into_inner()).await?;

        Ok(())
    }

    fn extract_request(&mut self) -> anyhow::Result<Request> {
        let mut request_bytes = Vec::new();

        self.tls.reader().read_to_end(&mut request_bytes)?;

        Request::from_bytes(&request_bytes[..]).map_err(|err| anyhow::anyhow!("{:?}", err))
    }
}
