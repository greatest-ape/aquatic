use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::common::*;
use crate::config::Config;


pub fn clean_connections(state: &State, config: &Config){
    let limit = Instant::now() - Duration::from_secs(
        config.cleaning.max_connection_age
    );

    state.connections.retain(|_, v| v.0 > limit);
    state.connections.shrink_to_fit();
}


pub fn clean_torrents(state: &State, config: &Config){
    let limit = Instant::now() - Duration::from_secs(
        config.cleaning.max_peer_age
    );

    state.torrents.retain(|_, torrent| {
        let num_seeders = &torrent.num_seeders;
        let num_leechers = &torrent.num_leechers;

        torrent.peers.retain(|_, peer| {
            let keep = peer.last_announce.0 > limit;

            if !keep {
                match peer.status {
                    PeerStatus::Seeding => {
                        num_seeders.fetch_sub(1, Ordering::SeqCst);
                    },
                    PeerStatus::Leeching => {
                        num_leechers.fetch_sub(1, Ordering::SeqCst);
                    },
                    _ => (),
                };
            }

            keep
        });

        !torrent.peers.is_empty()
    });

    state.torrents.shrink_to_fit();
}