use histogram::Histogram;

use aquatic_common::access_list::AccessListMode;

use crate::{common::*, config::Config};

pub fn update_access_list(config: &Config, state: &State) {
    match config.access_list.mode {
        AccessListMode::Require | AccessListMode::Forbid => {
            if let Err(err) = state
                .access_list
                .update_from_path(&config.access_list.path)
            {
                ::log::error!("Couldn't update access list: {:?}", err);
            }
        }
        AccessListMode::Ignore => {}
    }
}

pub fn print_statistics(state: &State) {
    let mut peers_per_torrent = Histogram::new();

    {
        let torrents = &mut state.torrent_maps.lock();

        for torrent in torrents.ipv4.values() {
            let num_peers = (torrent.num_seeders + torrent.num_leechers) as u64;

            if let Err(err) = peers_per_torrent.increment(num_peers) {
                eprintln!("error incrementing peers_per_torrent histogram: {}", err)
            }
        }
        for torrent in torrents.ipv6.values() {
            let num_peers = (torrent.num_seeders + torrent.num_leechers) as u64;

            if let Err(err) = peers_per_torrent.increment(num_peers) {
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
