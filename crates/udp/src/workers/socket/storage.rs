use std::collections::BTreeMap;

use hashbrown::HashMap;
use slab::Slab;

use aquatic_common::{SecondsSinceServerStart, ValidUntil};
use aquatic_udp_protocol::*;

use crate::common::*;
use crate::config::Config;

#[derive(Debug)]
pub struct PendingScrapeResponseSlabEntry {
    num_pending: usize,
    valid_until: ValidUntil,
    torrent_stats: BTreeMap<usize, TorrentScrapeStatistics>,
    transaction_id: TransactionId,
}

#[derive(Default)]
pub struct PendingScrapeResponseSlab(Slab<PendingScrapeResponseSlabEntry>);

impl PendingScrapeResponseSlab {
    pub fn prepare_split_requests(
        &mut self,
        config: &Config,
        request: ScrapeRequest,
        valid_until: ValidUntil,
    ) -> impl IntoIterator<Item = (SwarmWorkerIndex, PendingScrapeRequest)> {
        let capacity = config.swarm_workers.min(request.info_hashes.len());
        let mut split_requests: HashMap<SwarmWorkerIndex, PendingScrapeRequest> =
            HashMap::with_capacity(capacity);

        if request.info_hashes.is_empty() {
            ::log::warn!(
                "Attempted to prepare PendingScrapeResponseSlab entry with zero info hashes"
            );

            return split_requests;
        }

        let vacant_entry = self.0.vacant_entry();
        let slab_key = vacant_entry.key();

        for (i, info_hash) in request.info_hashes.into_iter().enumerate() {
            let split_request = split_requests
                .entry(SwarmWorkerIndex::from_info_hash(config, info_hash))
                .or_insert_with(|| PendingScrapeRequest {
                    slab_key,
                    info_hashes: BTreeMap::new(),
                });

            split_request.info_hashes.insert(i, info_hash);
        }

        vacant_entry.insert(PendingScrapeResponseSlabEntry {
            num_pending: split_requests.len(),
            valid_until,
            torrent_stats: Default::default(),
            transaction_id: request.transaction_id,
        });

        split_requests
    }

    pub fn add_and_get_finished(
        &mut self,
        response: &PendingScrapeResponse,
    ) -> Option<ScrapeResponse> {
        let finished = if let Some(entry) = self.0.get_mut(response.slab_key) {
            entry.num_pending -= 1;

            entry.torrent_stats.extend(response.torrent_stats.iter());

            entry.num_pending == 0
        } else {
            ::log::warn!(
                "PendingScrapeResponseSlab.add didn't find entry for key {:?}",
                response.slab_key
            );

            false
        };

        if finished {
            let entry = self.0.remove(response.slab_key);

            Some(ScrapeResponse {
                transaction_id: entry.transaction_id,
                torrent_stats: entry.torrent_stats.into_values().collect(),
            })
        } else {
            None
        }
    }

    pub fn clean(&mut self, now: SecondsSinceServerStart) {
        self.0.retain(|k, v| {
            if v.valid_until.valid(now) {
                true
            } else {
                ::log::warn!(
                    "Unconsumed PendingScrapeResponseSlab entry. {:?}: {:?}",
                    k,
                    v
                );

                false
            }
        });

        self.0.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use aquatic_common::ServerStartInstant;
    use quickcheck::TestResult;
    use quickcheck_macros::quickcheck;

    use super::*;

    #[quickcheck]
    fn test_pending_scrape_response_slab(
        request_data: Vec<(i32, i64, u8)>,
        swarm_workers: u8,
    ) -> TestResult {
        if swarm_workers == 0 {
            return TestResult::discard();
        }

        let config = Config {
            swarm_workers: swarm_workers as usize,
            ..Default::default()
        };

        let valid_until = ValidUntil::new(ServerStartInstant::new(), 1);

        let mut map = PendingScrapeResponseSlab::default();

        let mut requests = Vec::new();

        for (t, c, b) in request_data {
            if b == 0 {
                return TestResult::discard();
            }

            let mut info_hashes = Vec::new();

            for i in 0..b {
                let info_hash = InfoHash([i; 20]);

                info_hashes.push(info_hash);
            }

            let request = ScrapeRequest {
                transaction_id: TransactionId(t.into()),
                connection_id: ConnectionId(c.into()),
                info_hashes,
            };

            requests.push(request);
        }

        let mut all_split_requests = Vec::new();

        for request in requests.iter() {
            let split_requests =
                map.prepare_split_requests(&config, request.to_owned(), valid_until);

            all_split_requests.push(
                split_requests
                    .into_iter()
                    .collect::<Vec<(SwarmWorkerIndex, PendingScrapeRequest)>>(),
            );
        }

        assert_eq!(map.0.len(), requests.len());

        let mut responses = Vec::new();

        for split_requests in all_split_requests {
            for (worker_index, split_request) in split_requests {
                assert!(worker_index.0 < swarm_workers as usize);

                let torrent_stats = split_request
                    .info_hashes
                    .into_iter()
                    .map(|(i, info_hash)| {
                        (
                            i,
                            TorrentScrapeStatistics {
                                seeders: NumberOfPeers(((info_hash.0[0]) as i32).into()),
                                leechers: NumberOfPeers(0.into()),
                                completed: NumberOfDownloads(0.into()),
                            },
                        )
                    })
                    .collect();

                let response = PendingScrapeResponse {
                    slab_key: split_request.slab_key,
                    torrent_stats,
                };

                if let Some(response) = map.add_and_get_finished(&response) {
                    responses.push(response);
                }
            }
        }

        assert!(map.0.is_empty());
        assert_eq!(responses.len(), requests.len());

        TestResult::from_bool(true)
    }
}
