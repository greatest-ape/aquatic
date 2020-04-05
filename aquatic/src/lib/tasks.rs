use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::common::*;


pub fn clean_connections(state: &State){
    let limit = Instant::now() - Duration::from_secs(300);

    state.connections.retain(|_, v| v.0 > limit);
}


pub fn clean_torrents(state: &State){
    let limit = Instant::now() - Duration::from_secs(1200);

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
}