use std::io::{BufReader, Cursor, Read};
use std::rc::Rc;
use std::sync::Arc;

use aquatic_http_protocol::request::Request;
use aquatic_http_protocol::response::Response;
use futures_lite::{AsyncReadExt, AsyncWriteExt, StreamExt};
use glommio::prelude::*;
use glommio::net::{TcpListener, TcpStream};
use glommio::channels::local_channel::{new_bounded, LocalReceiver, LocalSender};
use glommio::task::JoinHandle;
use rustls::{IoState, ServerConnection};
use slab::Slab;

use crate::config::Config;

struct ConnectionReference {
    response_sender: LocalSender<Response>,
    handle: JoinHandle<()>,
}

struct Connection {
    response_receiver: LocalReceiver<Response>,
    tls: ServerConnection,
    stream: TcpStream,
    index: usize,
}

pub async fn run_socket_worker(
    config: Config,
) {
    let tls_config = Arc::new(create_tls_config(&config));
    let config = Rc::new(config);

    let listener = TcpListener::bind(config.network.address).expect("bind socket");

    let mut connection_slab: Slab<ConnectionReference> = Slab::new();

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let (response_sender, response_receiver) = new_bounded(1);

                let entry = connection_slab.vacant_entry();

                let conn = Connection {
                    response_receiver,
                    tls: ServerConnection::new(tls_config.clone()).unwrap(),
                    stream,
                    index: entry.key(),
                };

                async fn handle_stream(mut conn: Connection) {
                    conn.handle_stream().await;
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

impl Connection {
    async fn handle_stream(&mut self){
        loop {
            while let Some(response) = self.response_receiver.stream().next().await {
                response.write(&mut self.tls.writer()).unwrap();

                let mut buf = Vec::new();
                let mut buf = Cursor::new(&mut buf);

                while self.tls.wants_write() {
                    self.tls.write_tls(&mut buf).unwrap();
                }

                self.stream.write_all(&buf.into_inner()).await.unwrap();
            }
        }
    }

    async fn handle_stream_handshake(&mut self) {
        let mut buf = [0u8; 1024];

        loop {
            match self.stream.read(&mut buf).await {
                Ok(ciphertext_bytes_read) => {
                    let mut cursor = Cursor::new(&buf[..ciphertext_bytes_read]);

                    match self.tls.read_tls(&mut cursor) {
                        Ok(plaintext_bytes_read) => {
                            match self.tls.process_new_packets() {
                                Ok(_) => {
                                    if ciphertext_bytes_read == 0 && plaintext_bytes_read == 0 {
                                        let mut request_bytes = Vec::new();

                                        self.tls.reader().read_to_end(&mut request_bytes);

                                        match Request::from_bytes(&request_bytes[..]) {
                                            Ok(request) => {
                                                ::log::info!("request read: {:?}", request);
                                            },
                                            Err(err) => {
                                                // TODO: send error response, close connection

                                                ::log::info!("Request::from_bytes: {:?}", err);

                                                break
                                            }
                                        }
                                    }
                                    // TODO: check for io_state.peer_has_closed
                                },
                                Err(err) => {
                                    // TODO: call write_tls
                                    ::log::info!("conn.process_new_packets: {:?}", err);

                                    break
                                }
                            }
                        },
                        Err(err) => {
                            ::log::info!("conn.read_tls: {:?}", err);
                        }
                    }
                },
                Err(err) => {
                    ::log::info!("stream.read: {:?}", err);
                }
            }
        }
    }



}
fn create_tls_config(
    config: &Config,
) -> rustls::ServerConfig {
    let mut certs = Vec::new();
    let mut private_key = None;

    use std::iter;
    use rustls_pemfile::{Item, read_one};

    let pemfile = Vec::new();
    let mut reader = BufReader::new(&pemfile[..]);

    for item in iter::from_fn(|| read_one(&mut reader).transpose()) {
        match item.unwrap() {
            Item::X509Certificate(cert) => {
                certs.push(rustls::Certificate(cert));
            },
            Item::RSAKey(key) | Item::PKCS8Key(key) => {
                if private_key.is_none(){
                    private_key = Some(rustls::PrivateKey(key));
                }
            }
        }
    }

    rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, private_key.expect("no private key"))
        .expect("bad certificate/key")
}