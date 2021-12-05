use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::enclose;
use glommio::prelude::*;
use glommio::timer::TimerActionRepeat;
use rand::{rngs::SmallRng, SeedableRng};

use aquatic_ws_protocol::*;

use crate::common::handlers::*;
use crate::common::*;
use crate::config::Config;

use super::SHARED_CHANNEL_SIZE;
use super::common::State;

pub async fn run_request_worker(
    config: Config,
    state: State,
    in_message_mesh_builder: MeshBuilder<(ConnectionMeta, InMessage), Partial>,
    out_message_mesh_builder: MeshBuilder<(ConnectionMeta, OutMessage), Partial>,
) {
    let (_, mut in_message_receivers) = in_message_mesh_builder.join(Role::Consumer).await.unwrap();
    let (out_message_senders, _) = out_message_mesh_builder.join(Role::Producer).await.unwrap();

    let out_message_senders = Rc::new(out_message_senders);

    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));
    let access_list = state.access_list;

    // Periodically clean torrents
    TimerActionRepeat::repeat(enclose!((config, torrents, access_list) move || {
        enclose!((config, torrents, access_list) move || async move {
            torrents.borrow_mut().clean(&config, &access_list);

            Some(Duration::from_secs(config.cleaning.torrent_cleaning_interval))
        })()
    }));

    let mut handles = Vec::new();

    for (_, receiver) in in_message_receivers.streams() {
        let handle = spawn_local(handle_request_stream(
            config.clone(),
            torrents.clone(),
            out_message_senders.clone(),
            receiver,
        ))
        .detach();

        handles.push(handle);
    }

    for handle in handles {
        handle.await;
    }
}

async fn handle_request_stream<S>(
    config: Config,
    torrents: Rc<RefCell<TorrentMaps>>,
    out_message_senders: Rc<Senders<(ConnectionMeta, OutMessage)>>,
    stream: S,
) where
    S: futures_lite::Stream<Item = (ConnectionMeta, InMessage)> + ::std::marker::Unpin,
{
    let rng = Rc::new(RefCell::new(SmallRng::from_entropy()));

    let max_peer_age = config.cleaning.max_peer_age;
    let peer_valid_until = Rc::new(RefCell::new(ValidUntil::new(max_peer_age)));

    TimerActionRepeat::repeat(enclose!((peer_valid_until) move || {
        enclose!((peer_valid_until) move || async move {
            *peer_valid_until.borrow_mut() = ValidUntil::new(max_peer_age);

            Some(Duration::from_secs(1))
        })()
    }));

    let config = &config;
    let torrents = &torrents;
    let peer_valid_until = &peer_valid_until;
    let rng = &rng;
    let out_message_senders = &out_message_senders;

    stream.for_each_concurrent(SHARED_CHANNEL_SIZE, move |(meta, in_message)| async move {
        let mut out_messages = Vec::new();

        match in_message {
            InMessage::AnnounceRequest(request) => handle_announce_request(
                &config,
                &mut rng.borrow_mut(),
                &mut torrents.borrow_mut(),
                &mut out_messages,
                peer_valid_until.borrow().to_owned(),
                meta,
                request,
            ),
            InMessage::ScrapeRequest(request) => handle_scrape_request(
                &config,
                &mut torrents.borrow_mut(),
                &mut out_messages,
                meta,
                request,
            ),
        };

        for (meta, out_message) in out_messages.drain(..) {
            ::log::info!("request worker trying to send OutMessage to socket worker");

            out_message_senders
                .send_to(meta.out_message_consumer_id.0, (meta, out_message))
                .await
                .expect("failed sending out_message to socket worker");

            ::log::info!("request worker sent OutMessage to socket worker");
        }
    }).await;
}
