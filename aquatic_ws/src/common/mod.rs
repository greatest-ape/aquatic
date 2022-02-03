pub mod handlers;

use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Instant;

use aquatic_common::access_list::{create_access_list_cache, AccessListArcSwap, AccessListCache};
use aquatic_common::{AHashIndexMap, CanonicalSocketAddr};

pub use aquatic_common::ValidUntil;

use aquatic_ws_protocol::*;

use crate::config::Config;

#[derive(Copy, Clone, Debug)]
pub struct PendingScrapeId(pub usize);

#[derive(Copy, Clone, Debug)]
pub struct ConsumerId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ConnectionId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct ConnectionMeta {
    /// Index of socket worker responsible for this connection. Required for
    /// sending back response through correct channel to correct worker.
    pub out_message_consumer_id: ConsumerId,
    pub connection_id: ConnectionId,
    pub peer_addr: CanonicalSocketAddr,
    pub pending_scrape_id: Option<PendingScrapeId>,
}

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

#[derive(Clone, Copy)]
pub struct Peer {
    pub connection_meta: ConnectionMeta,
    pub status: PeerStatus,
    pub valid_until: ValidUntil,
}

pub type PeerMap = AHashIndexMap<PeerId, Peer>;

pub struct TorrentData {
    pub peers: PeerMap,
    pub num_seeders: usize,
    pub num_leechers: usize,
}

impl Default for TorrentData {
    #[inline]
    fn default() -> Self {
        Self {
            peers: Default::default(),
            num_seeders: 0,
            num_leechers: 0,
        }
    }
}

pub type TorrentMap = AHashIndexMap<InfoHash, TorrentData>;

#[derive(Default)]
pub struct TorrentMaps {
    pub ipv4: TorrentMap,
    pub ipv6: TorrentMap,
}

impl TorrentMaps {
    pub fn clean(&mut self, config: &Config, access_list: &Arc<AccessListArcSwap>) {
        let mut access_list_cache = create_access_list_cache(access_list);

        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv4);
        Self::clean_torrent_map(config, &mut access_list_cache, &mut self.ipv6);
    }

    fn clean_torrent_map(
        config: &Config,
        access_list_cache: &mut AccessListCache,
        torrent_map: &mut TorrentMap,
    ) {
        let now = Instant::now();

        torrent_map.retain(|info_hash, torrent_data| {
            if !access_list_cache
                .load()
                .allows(config.access_list.mode, &info_hash.0)
            {
                return false;
            }

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

pub fn create_tls_config(config: &Config) -> anyhow::Result<rustls::ServerConfig> {
    let certs = {
        let f = File::open(&config.network.tls_certificate_path)?;
        let mut f = BufReader::new(f);

        rustls_pemfile::certs(&mut f)?
            .into_iter()
            .map(|bytes| rustls::Certificate(bytes))
            .collect()
    };

    let private_key = {
        let f = File::open(&config.network.tls_private_key_path)?;
        let mut f = BufReader::new(f);

        rustls_pemfile::pkcs8_private_keys(&mut f)?
            .first()
            .map(|bytes| rustls::PrivateKey(bytes.clone()))
            .ok_or(anyhow::anyhow!("No private keys in file"))?
    };

    let tls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, private_key)?;

    Ok(tls_config)
}
