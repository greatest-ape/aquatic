mod storage;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use aquatic_ws_protocol::incoming::InMessage;
use aquatic_ws_protocol::outgoing::OutMessage;
use futures::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::enclose;
use glommio::prelude::*;
use glommio::timer::TimerActionRepeat;
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_common::ServerStartInstant;

use crate::common::*;
use crate::config::Config;
use crate::SHARED_IN_CHANNEL_SIZE;

use self::storage::TorrentMaps;

pub async fn run_swarm_worker(
    config: Config,
    state: State,
    control_message_mesh_builder: MeshBuilder<SwarmControlMessage, Partial>,
    in_message_mesh_builder: MeshBuilder<(InMessageMeta, InMessage), Partial>,
    out_message_mesh_builder: MeshBuilder<(OutMessageMeta, OutMessage), Partial>,
    server_start_instant: ServerStartInstant,
    worker_index: usize,
) -> anyhow::Result<()> {
    let (_, mut control_message_receivers) = control_message_mesh_builder
        .join(Role::Consumer)
        .await
        .map_err(|err| anyhow::anyhow!("join control message mesh: {:#}", err))?;
    let (_, mut in_message_receivers) = in_message_mesh_builder
        .join(Role::Consumer)
        .await
        .map_err(|err| anyhow::anyhow!("join in message mesh: {:#}", err))?;
    let (out_message_senders, _) = out_message_mesh_builder
        .join(Role::Producer)
        .await
        .map_err(|err| anyhow::anyhow!("join out message mesh: {:#}", err))?;

    let out_message_senders = Rc::new(out_message_senders);

    let torrents = Rc::new(RefCell::new(TorrentMaps::new(worker_index)));
    let access_list = state.access_list;

    // Periodically clean torrents
    TimerActionRepeat::repeat(enclose!((config, torrents, access_list) move || {
        enclose!((config, torrents, access_list) move || async move {
            torrents.borrow_mut().clean(&config, &access_list, server_start_instant);

            Some(Duration::from_secs(config.cleaning.torrent_cleaning_interval))
        })()
    }));

    // Periodically update torrent count metrics
    #[cfg(feature = "metrics")]
    TimerActionRepeat::repeat(enclose!((config, torrents) move || {
        enclose!((config, torrents) move || async move {
            torrents.borrow_mut().update_torrent_count_metrics();

            Some(Duration::from_secs(config.metrics.torrent_count_update_interval))
        })()
    }));

    let mut handles = Vec::new();

    for (_, receiver) in control_message_receivers.streams() {
        let handle =
            spawn_local(handle_control_message_stream(torrents.clone(), receiver)).detach();

        handles.push(handle);
    }

    for (_, receiver) in in_message_receivers.streams() {
        let handle = spawn_local(handle_request_stream(
            config.clone(),
            torrents.clone(),
            server_start_instant,
            out_message_senders.clone(),
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

async fn handle_control_message_stream<S>(torrents: Rc<RefCell<TorrentMaps>>, mut stream: S)
where
    S: futures_lite::Stream<Item = SwarmControlMessage> + ::std::marker::Unpin,
{
    while let Some(message) = stream.next().await {
        match message {
            SwarmControlMessage::ConnectionClosed {
                ip_version,
                announced_info_hashes,
            } => {
                let mut torrents = torrents.borrow_mut();

                for (info_hash, peer_id) in announced_info_hashes {
                    torrents.handle_connection_closed(info_hash, peer_id, ip_version);
                }
            }
        }
    }
}

async fn handle_request_stream<S>(
    config: Config,
    torrents: Rc<RefCell<TorrentMaps>>,
    server_start_instant: ServerStartInstant,
    out_message_senders: Rc<Senders<(OutMessageMeta, OutMessage)>>,
    stream: S,
) where
    S: futures_lite::Stream<Item = (InMessageMeta, InMessage)> + ::std::marker::Unpin,
{
    let rng = Rc::new(RefCell::new(SmallRng::from_entropy()));
    let config = &config;
    let torrents = &torrents;
    let rng = &rng;
    let out_message_senders = &out_message_senders;

    stream
        .for_each_concurrent(
            SHARED_IN_CHANNEL_SIZE,
            move |(meta, in_message)| async move {
                let mut out_messages = Vec::new();

                match in_message {
                    InMessage::AnnounceRequest(request) => {
                        torrents.borrow_mut().handle_announce_request(
                            config,
                            &mut rng.borrow_mut(),
                            &mut out_messages,
                            server_start_instant,
                            meta,
                            request,
                        )
                    }
                    InMessage::ScrapeRequest(request) => torrents
                        .borrow_mut()
                        .handle_scrape_request(config, &mut out_messages, meta, request),
                };

                for (meta, out_message) in out_messages {
                    out_message_senders
                        .send_to(meta.out_message_consumer_id.0 as usize, (meta, out_message))
                        .await
                        .expect("failed sending out_message to socket worker");

                    ::log::debug!("swarm worker sent OutMessage to socket worker");
                }
            },
        )
        .await;
}
