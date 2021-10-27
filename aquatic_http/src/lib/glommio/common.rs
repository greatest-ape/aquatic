use std::borrow::Borrow;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;

use aquatic_common::access_list::AccessList;
use futures_lite::AsyncBufReadExt;
use glommio::io::{BufferedFile, StreamReaderBuilder};
use glommio::prelude::*;

use aquatic_http_protocol::{
    request::{AnnounceRequest, ScrapeRequest},
    response::{AnnounceResponse, ScrapeResponse},
};

use crate::config::Config;

#[derive(Copy, Clone, Debug)]
pub struct ConsumerId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ConnectionId(pub usize);

#[derive(Debug)]
pub enum ChannelRequest {
    Announce {
        request: AnnounceRequest,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
        response_consumer_id: ConsumerId,
    },
    Scrape {
        request: ScrapeRequest,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
        response_consumer_id: ConsumerId,
    },
}

#[derive(Debug)]
pub enum ChannelResponse {
    Announce {
        response: AnnounceResponse,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
    },
    Scrape {
        response: ScrapeResponse,
        peer_addr: SocketAddr,
        connection_id: ConnectionId,
    },
}

impl ChannelResponse {
    pub fn get_connection_id(&self) -> ConnectionId {
        match self {
            Self::Announce { connection_id, .. } => *connection_id,
            Self::Scrape { connection_id, .. } => *connection_id,
        }
    }
    pub fn get_peer_addr(&self) -> SocketAddr {
        match self {
            Self::Announce { peer_addr, .. } => *peer_addr,
            Self::Scrape { peer_addr, .. } => *peer_addr,
        }
    }
}

pub async fn update_access_list<C: Borrow<Config>>(
    config: C,
    access_list: Rc<RefCell<AccessList>>,
) {
    if config.borrow().access_list.mode.is_on() {
        match BufferedFile::open(&config.borrow().access_list.path).await {
            Ok(file) => {
                let mut reader = StreamReaderBuilder::new(file).build();
                let mut new_access_list = AccessList::default();

                loop {
                    let mut buf = String::with_capacity(42);

                    match reader.read_line(&mut buf).await {
                        Ok(_) => {
                            if let Err(err) = new_access_list.insert_from_line(&buf) {
                                ::log::error!(
                                    "Couln't parse access list line '{}': {:?}",
                                    buf,
                                    err
                                );
                            }
                        }
                        Err(err) => {
                            ::log::error!("Couln't read access list line {:?}", err);

                            break;
                        }
                    }

                    yield_if_needed().await;
                }

                *access_list.borrow_mut() = new_access_list;
            }
            Err(err) => {
                ::log::error!("Couldn't open access list file: {:?}", err)
            }
        };
    }
}
