use std::time::Instant;

use crate::common::*;


// identical to ws version
pub fn clean_torrents(state: &State){
    fn clean_torrent_map(
        torrent_map: &mut TorrentMap,
    ){
        let now = Instant::now();

        torrent_map.retain(|_, torrent_data| {
            torrent_data.peers.retain(|_, peer| {
                peer.valid_until.0 >= now
            });

            !torrent_data.peers.is_empty()
        });

        torrent_map.shrink_to_fit();
    }

    let mut torrent_maps = state.torrent_maps.lock();

    clean_torrent_map(&mut torrent_maps.ipv4);
    clean_torrent_map(&mut torrent_maps.ipv6);
}