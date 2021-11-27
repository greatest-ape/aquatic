use std::sync::Arc;

use aquatic_common::access_list::AccessListArcSwap;
use aquatic_ws_protocol::*;
use crossbeam_channel::{Receiver, Sender};
use log::error;
use mio::Token;
use parking_lot::Mutex;

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

pub type InMessageSender = Sender<(ConnectionMeta, InMessage)>;
pub type InMessageReceiver = Receiver<(ConnectionMeta, InMessage)>;
pub type OutMessageReceiver = Receiver<(ConnectionMeta, OutMessage)>;

#[derive(Clone)]
pub struct OutMessageSender(Vec<Sender<(ConnectionMeta, OutMessage)>>);

impl OutMessageSender {
    pub fn new(senders: Vec<Sender<(ConnectionMeta, OutMessage)>>) -> Self {
        Self(senders)
    }

    #[inline]
    pub fn send(&self, meta: ConnectionMeta, message: OutMessage) {
        if let Err(err) = self.0[meta.out_message_consumer_id.0].send((meta, message)) {
            error!("OutMessageSender: couldn't send message: {:?}", err);
        }
    }
}

pub type SocketWorkerStatus = Option<Result<(), String>>;
pub type SocketWorkerStatuses = Arc<Mutex<Vec<SocketWorkerStatus>>>;
