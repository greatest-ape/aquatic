use std::sync::Arc;

use aquatic_common::access_list::AccessListArcSwap;
use crossbeam_channel::{Receiver, Sender};
use log::error;
use mio::Token;
use parking_lot::Mutex;

pub use aquatic_common::{convert_ipv4_mapped_ipv6, ValidUntil};

use aquatic_http_protocol::request::Request;
use aquatic_http_protocol::response::Response;

use crate::common::*;

pub const LISTENER_TOKEN: Token = Token(0);
pub const CHANNEL_TOKEN: Token = Token(1);

#[derive(Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
    pub torrent_maps: Arc<Mutex<TorrentMaps>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            access_list: Arc::new(Default::default()),
            torrent_maps: Arc::new(Mutex::new(TorrentMaps::default())),
        }
    }
}

pub type RequestChannelSender = Sender<(ConnectionMeta, Request)>;
pub type RequestChannelReceiver = Receiver<(ConnectionMeta, Request)>;
pub type ResponseChannelReceiver = Receiver<(ConnectionMeta, Response)>;

#[derive(Clone)]
pub struct ResponseChannelSender {
    senders: Vec<Sender<(ConnectionMeta, Response)>>,
}

impl ResponseChannelSender {
    pub fn new(senders: Vec<Sender<(ConnectionMeta, Response)>>) -> Self {
        Self { senders }
    }

    #[inline]
    pub fn send(&self, meta: ConnectionMeta, message: Response) {
        if let Err(err) = self.senders[meta.worker_index].send((meta, message)) {
            error!("ResponseChannelSender: couldn't send message: {:?}", err);
        }
    }
}

pub type SocketWorkerStatus = Option<Result<(), String>>;
pub type SocketWorkerStatuses = Arc<Mutex<Vec<SocketWorkerStatus>>>;
