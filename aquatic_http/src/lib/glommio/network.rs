use std::io::{BufReader, Cursor, Read};
use std::rc::Rc;
use std::sync::Arc;

use aquatic_http_protocol::request::Request;
use futures_lite::{AsyncReadExt, StreamExt};
use glommio::prelude::*;
use glommio::net::{TcpListener, TcpStream};
use rustls::{IoState, ServerConnection};

use crate::config::Config;

pub async fn run_socket_worker(
    config: Config,
) {
    let tlsConfig = Arc::new(create_tls_config(&config));
    let config = Rc::new(config);

    let listener = TcpListener::bind(config.network.address).expect("bind socket");

    let mut incoming = listener.incoming();

    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                spawn_local(handle_stream(config.clone(), tlsConfig.clone(), stream)).detach();
            },
            Err(err) => {
                ::log::error!("accept connection: {:?}", err);
            }
        }
        
    }
}

async fn handle_stream(
    config: Rc<Config>,
    tlsConfig: Arc<rustls::ServerConfig>,
    mut stream: TcpStream,
){
    let mut buf = [0u8; 1024];
    let mut conn = ServerConnection::new(tlsConfig).unwrap();

    loop {
        match stream.read(&mut buf).await {
            Ok(ciphertext_bytes_read) => {
                let mut cursor = Cursor::new(&buf[..ciphertext_bytes_read]);

                match conn.read_tls(&mut cursor) {
                    Ok(plaintext_bytes_read) => {
                        match conn.process_new_packets() {
                            Ok(_) => {
                                if ciphertext_bytes_read == 0 && plaintext_bytes_read == 0 {
                                    let mut request_bytes = Vec::new();

                                    conn.reader().read_to_end(&mut request_bytes);

                                    match Request::from_bytes(&request_bytes[..]) {
                                        Ok(request) => {

                                        },
                                        Err(err) => {
                                            // TODO: return error response, close connection
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