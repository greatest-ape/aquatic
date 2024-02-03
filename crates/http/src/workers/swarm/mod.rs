mod storage;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use futures_lite::{Stream, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role};
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use rand::prelude::SmallRng;
use rand::SeedableRng;

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
    let peer_valid_until = Rc::new(RefCell::new(ValidUntil::new(
        server_start_instant,
        max_peer_age,
    )));

    // Periodically update peer_valid_until
    TimerActionRepeat::repeat(enclose!((peer_valid_until) move || {
        enclose!((peer_valid_until) move || async move {
            *peer_valid_until.borrow_mut() = ValidUntil::new(server_start_instant, max_peer_age);

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
    let mut rng = SmallRng::from_entropy();

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
