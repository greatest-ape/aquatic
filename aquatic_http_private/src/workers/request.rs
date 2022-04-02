use std::cell::RefCell;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::rc::Rc;
use std::time::Instant;

use tokio::sync::mpsc::Receiver;
use tokio::task::LocalSet;
use tokio::time;

use aquatic_common::{AmortizedIndexMap, CanonicalSocketAddr, ValidUntil};
use aquatic_http_protocol::common::{AnnounceEvent, InfoHash, PeerId};
use aquatic_http_protocol::request::AnnounceRequest;
use aquatic_http_protocol::response::{FailureResponse, Response, ResponsePeer};

use crate::common::ChannelAnnounceRequest;
use crate::config::Config;

pub trait Ip: ::std::fmt::Debug + Copy + Eq + ::std::hash::Hash {}

impl Ip for Ipv4Addr {}
impl Ip for Ipv6Addr {}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum PeerStatus {
    Seeding,
    Leeching,
    Stopped,
}

impl PeerStatus {
    /// Determine peer status from announce event and number of bytes left.
    ///
    /// Likely, the last branch will be taken most of the time.
    #[inline]
    pub fn from_event_and_bytes_left(event: AnnounceEvent, opt_bytes_left: Option<usize>) -> Self {
        if let AnnounceEvent::Stopped = event {
            Self::Stopped
        } else if let Some(0) = opt_bytes_left {
            Self::Seeding
        } else {
            Self::Leeching
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Peer<I: Ip> {
    pub ip_address: I,
    pub port: u16,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}

impl<I: Ip> Peer<I> {
    pub fn to_response_peer(&self) -> ResponsePeer<I> {
        ResponsePeer {
            ip_address: self.ip_address,
            port: self.port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerMapKey<I: Ip> {
    pub peer_id: PeerId,
    pub ip: I,
}

pub type PeerMap<I> = AmortizedIndexMap<PeerMapKey<I>, Peer<I>>;

pub struct TorrentData<I: Ip> {
    pub peers: PeerMap<I>,
    pub num_seeders: usize,
    pub num_leechers: usize,
}

impl<I: Ip> Default for TorrentData<I> {
    #[inline]
    fn default() -> Self {
        Self {
            peers: Default::default(),
            num_seeders: 0,
            num_leechers: 0,
        }
    }
}

pub type TorrentMap<I> = AmortizedIndexMap<InfoHash, TorrentData<I>>;

#[derive(Default)]
pub struct TorrentMaps {
    pub ipv4: TorrentMap<Ipv4Addr>,
    pub ipv6: TorrentMap<Ipv6Addr>,
}

impl TorrentMaps {
    pub fn clean(&mut self) {
        Self::clean_torrent_map(&mut self.ipv4);
        Self::clean_torrent_map(&mut self.ipv6);
    }

    fn clean_torrent_map<I: Ip>(torrent_map: &mut TorrentMap<I>) {
        let now = Instant::now();

        torrent_map.retain(|_, torrent_data| {
            let num_seeders = &mut torrent_data.num_seeders;
            let num_leechers = &mut torrent_data.num_leechers;

            torrent_data.peers.retain(|_, peer| {
                let keep = peer.valid_until.0 >= now;

                if !keep {
                    match peer.status {
                        PeerStatus::Seeding => {
                            *num_seeders -= 1;
                        }
                        PeerStatus::Leeching => {
                            *num_leechers -= 1;
                        }
                        _ => (),
                    };
                }

                keep
            });

            !torrent_data.peers.is_empty()
        });

        torrent_map.shrink_to_fit();
    }
}

pub fn run_request_worker(
    config: Config,
    request_receiver: Receiver<ChannelAnnounceRequest>,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_inner(config, request_receiver))?;

    Ok(())
}

async fn run_inner(
    config: Config,
    mut request_receiver: Receiver<ChannelAnnounceRequest>,
) -> anyhow::Result<()> {
    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));

    LocalSet::new().spawn_local(periodically_clean_torrents(
        config.clone(),
        torrents.clone(),
    ));

    loop {
        let request = request_receiver
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("request channel closed"))?;

        let response = handle_announce_request(
            &config,
            &torrents,
            request.source_addr,
            request.request.into(),
        );

        let _ = request.response_sender.send(response);
    }
}

fn handle_announce_request(
    config: &Config,
    torrents: &Rc<RefCell<TorrentMaps>>,
    source_addr: CanonicalSocketAddr,
    request: AnnounceRequest,
) -> Response {
    Response::Failure(FailureResponse::new("actually successful"))
}

async fn periodically_clean_torrents(config: Config, torrents: Rc<RefCell<TorrentMaps>>) {
    let mut interval = time::interval(time::Duration::from_secs(
        config.cleaning.torrent_cleaning_interval,
    ));

    loop {
        interval.tick().await;

        torrents.borrow_mut().clean();
    }
}
