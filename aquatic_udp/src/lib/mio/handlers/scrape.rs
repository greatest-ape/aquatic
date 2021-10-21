use std::net::SocketAddr;
use std::vec::Drain;

use parking_lot::MutexGuard;

use aquatic_common::convert_ipv4_mapped_ipv6;
use aquatic_udp_protocol::*;

use crate::mio::common::*;

use crate::common::*;

#[inline]
pub fn handle_scrape_requests(
    torrents: &mut MutexGuard<TorrentMaps>,
    requests: Drain<(ScrapeRequest, SocketAddr)>,
    responses: &mut Vec<(ConnectedResponse, SocketAddr)>,
) {
    let empty_stats = create_torrent_scrape_statistics(0, 0);

    responses.extend(requests.map(|(request, src)| {
        let mut stats: Vec<TorrentScrapeStatistics> = Vec::with_capacity(request.info_hashes.len());

        let peer_ip = convert_ipv4_mapped_ipv6(src.ip());

        if peer_ip.is_ipv4() {
            for info_hash in request.info_hashes.iter() {
                if let Some(torrent_data) = torrents.ipv4.get(info_hash) {
                    stats.push(create_torrent_scrape_statistics(
                        torrent_data.num_seeders as i32,
                        torrent_data.num_leechers as i32,
                    ));
                } else {
                    stats.push(empty_stats);
                }
            }
        } else {
            for info_hash in request.info_hashes.iter() {
                if let Some(torrent_data) = torrents.ipv6.get(info_hash) {
                    stats.push(create_torrent_scrape_statistics(
                        torrent_data.num_seeders as i32,
                        torrent_data.num_leechers as i32,
                    ));
                } else {
                    stats.push(empty_stats);
                }
            }
        }

        let response = ConnectedResponse::Scrape(ScrapeResponse {
            transaction_id: request.transaction_id,
            torrent_stats: stats,
        });

        (response, src)
    }));
}

#[inline(always)]
fn create_torrent_scrape_statistics(seeders: i32, leechers: i32) -> TorrentScrapeStatistics {
    TorrentScrapeStatistics {
        seeders: NumberOfPeers(seeders),
        completed: NumberOfDownloads(0), // No implementation planned
        leechers: NumberOfPeers(leechers),
    }
}
