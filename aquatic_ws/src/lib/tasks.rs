use std::time::Instant;

use histogram::Histogram;

use crate::common::*;


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


pub fn print_statistics(state: &State){
    let mut peers_per_torrent = Histogram::new();

    {
        let torrents = &mut state.torrent_maps.lock();

        for torrent in torrents.ipv4.values(){
            let num_peers = (torrent.num_seeders + torrent.num_leechers) as u64;

            if let Err(err) = peers_per_torrent.increment(num_peers){
                eprintln!("error incrementing peers_per_torrent histogram: {}", err)
            }
        }
        for torrent in torrents.ipv6.values(){
            let num_peers = (torrent.num_seeders + torrent.num_leechers) as u64;

            if let Err(err) = peers_per_torrent.increment(num_peers){
                eprintln!("error incrementing peers_per_torrent histogram: {}", err)
            }
        }
    }

    if peers_per_torrent.entries() != 0 {
        println!(
            "peers per torrent: min: {}, p50: {}, p75: {}, p90: {}, p99: {}, p999: {}, max: {}",
            peers_per_torrent.minimum().unwrap(),
            peers_per_torrent.percentile(50.0).unwrap(),
            peers_per_torrent.percentile(75.0).unwrap(),
            peers_per_torrent.percentile(90.0).unwrap(),
            peers_per_torrent.percentile(99.0).unwrap(),
            peers_per_torrent.percentile(99.9).unwrap(),
            peers_per_torrent.maximum().unwrap(),
        );
    }
}