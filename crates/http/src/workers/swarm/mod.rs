mod storage;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use futures_lite::{Stream, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role};
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use rand::{make_rng, rngs::SmallRng};

use aquatic_common::{ServerStartInstant, ValidUntil};

use crate::common::*;
use crate::config::Config;

use self::storage::TorrentMaps;

pub async fn run_swarm_worker(
    config: Config,
    state: State,
    request_mesh_builder: MeshBuilder<ChannelRequest, Partial>,
    server_start_instant: ServerStartInstant,
    worker_index: usize,
) -> anyhow::Result<()> {
    let (_, mut request_receivers) = request_mesh_builder
        .join(Role::Consumer)
        .await
        .map_err(|err| anyhow::anyhow!("join request mesh: {:#}", err))?;

    let torrents = Rc::new(RefCell::new(TorrentMaps::new(worker_index)));
    let access_list = state.access_list;

    // Periodically clean torrents
    TimerActionRepeat::repeat(enclose!((config, torrents, access_list) move || {
        enclose!((config, torrents, access_list) move || async move {
            torrents.borrow_mut().clean(&config, &access_list, server_start_instant);

            Some(Duration::from_secs(config.cleaning.torrent_cleaning_interval))
        })()
    }));

    let max_peer_age = config.cleaning.max_peer_age;
    let peer_valid_until = Rc::new(RefCell::new(
        ValidUntil::new(server_start_instant, max_peer_age)
            .expect("Could not create initial ValidUntil due to monotonicity error"),
    ));

    // Periodically update peer_valid_until
    TimerActionRepeat::repeat(enclose!((peer_valid_until) move || {
        enclose!((peer_valid_until) move || async move {
            if let Some(valid_until) = ValidUntil::new(server_start_instant, max_peer_age) {
                *peer_valid_until.borrow_mut() = valid_until;
            } else {
                ::log::warn!("Could not update peer_valid_until due to monotonicity error. Peers may be removed earlier than they should.");
            }

            Some(Duration::from_secs(1))
        })()
    }));

    // Periodically update torrent count metrics
    #[cfg(feature = "metrics")]
    TimerActionRepeat::repeat(enclose!((config, torrents) move || {
        enclose!((config, torrents) move || async move {
            torrents.borrow_mut().update_torrent_metrics();

            Some(Duration::from_secs(config.metrics.torrent_count_update_interval))
        })()
    }));

    // Periodically update torrent info-hash API
    #[cfg(feature = "info_hash_api")]
    if config.info_hash_api.is_ipv4_enabled || config.info_hash_api.is_ipv6_enabled {
        TimerActionRepeat::repeat(enclose!((config, torrents) move || {
            enclose!((config, torrents) move || async move {
                fn update<'a>(info_hash_table_bytes: impl Iterator<Item = &'a [u8; 20]>, file_path: &std::path::PathBuf) -> usize {
                    let mut total = 0;
                    match std::fs::File::create(file_path) {
                        Ok(file) => {
                            use std::io::Write;
                            let mut w = std::io::BufWriter::new(file);
                            for b in info_hash_table_bytes {
                                match w.write_all(b) {
                                    Ok(()) => total += 1,
                                    Err(e) => {
                                        ::log::error!("Info-hash bytes write error: {e}");
                                        break;
                                    }
                                }
                            }
                            if let Err(e) = w.flush() {
                                ::log::error!("Could not flash info-hash bytes: {e}");
                            }
                        }
                        Err(e) => ::log::error!(
                            "Could not open info-hash API file `{}` to write: {e}",
                            file_path.to_string_lossy()
                        )
                    }
                    total
                }
                let t = torrents.borrow();
                if config.info_hash_api.is_ipv4_enabled {
                    ::log::debug!(
                        "IPv4 info-hash API file updated ({} total).",
                        update(t.ipv4_info_hash_table_bytes(), &config.info_hash_api.ipv4_file_path)
                    )
                }
                if config.info_hash_api.is_ipv6_enabled {
                    ::log::debug!(
                        "IPv6 info-hash API file updated ({} total).",
                        update(t.ipv6_info_hash_table_bytes(), &config.info_hash_api.ipv6_file_path)
                    )
                }
                ::log::debug!(
                    "Update info-hash API file completed; await {} seconds to renew...",
                    config.info_hash_api.update_interval
                );
                Some(Duration::from_secs(config.info_hash_api.update_interval))
            })()
        }));
    }

    let mut handles = Vec::new();

    for (_, receiver) in request_receivers.streams() {
        let handle = spawn_local(handle_request_stream(
            config.clone(),
            torrents.clone(),
            peer_valid_until.clone(),
            receiver,
        ))
        .detach();

        handles.push(handle);
    }

    for handle in handles {
        handle.await;
    }

    Ok(())
}

async fn handle_request_stream<S>(
    config: Config,
    torrents: Rc<RefCell<TorrentMaps>>,
    peer_valid_until: Rc<RefCell<ValidUntil>>,
    mut stream: S,
) where
    S: Stream<Item = ChannelRequest> + ::std::marker::Unpin,
{
    let mut rng: SmallRng = make_rng();

    while let Some(channel_request) = stream.next().await {
        match channel_request {
            ChannelRequest::Announce {
                request,
                peer_addr,
                response_sender,
            } => {
                let response = torrents.borrow_mut().handle_announce_request(
                    &config,
                    &mut rng,
                    peer_valid_until.borrow().to_owned(),
                    peer_addr,
                    request,
                );

                if let Err(err) = response_sender.connect().await.send(response).await {
                    ::log::error!("swarm worker could not send announce response: {:#}", err);
                }
            }
            ChannelRequest::Scrape {
                request,
                peer_addr,
                response_sender,
            } => {
                let response = torrents
                    .borrow_mut()
                    .handle_scrape_request(&config, peer_addr, request);

                if let Err(err) = response_sender.connect().await.send(response).await {
                    ::log::error!("swarm worker could not send scrape response: {:#}", err);
                }
            }
        };
    }
}
