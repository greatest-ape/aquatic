use std::time::Instant;

use crate::common::*;


pub fn clean_torrents(state: &State){
    let mut torrents = state.torrents.lock();

    let now = Instant::now();

    torrents.retain(|_, torrent_data| {
        torrent_data.peers.retain(|_, peer| {
            peer.valid_until.0 >= now
        });

        !torrent_data.peers.is_empty()
    });

    torrents.shrink_to_fit();
}