use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use aquatic_common::access_list::AccessList;
use futures_lite::{Stream, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use rand::prelude::SmallRng;
use rand::SeedableRng;

use crate::common::handlers::handle_announce_request;
use crate::common::handlers::*;
use crate::common::*;
use crate::config::Config;

use super::common::*;

pub async fn run_request_worker(
    config: Config,
    request_mesh_builder: MeshBuilder<ChannelRequest, Partial>,
    response_mesh_builder: MeshBuilder<ChannelResponse, Partial>,
    access_list: AccessList,
) {
    let (_, mut request_receivers) = request_mesh_builder.join(Role::Consumer).await.unwrap();
    let (response_senders, _) = response_mesh_builder.join(Role::Producer).await.unwrap();

    let response_senders = Rc::new(response_senders);

    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));
    let access_list = Rc::new(RefCell::new(access_list));

    // Periodically clean torrents and update access list
    TimerActionRepeat::repeat(enclose!((config, torrents, access_list) move || {
        enclose!((config, torrents, access_list) move || async move {
            // update_access_list(config.clone(), access_list.clone()).await;

            torrents.borrow_mut().clean(&config, &*access_list.borrow());

            Some(Duration::from_secs(config.cleaning.interval))
        })()
    }));

    let mut handles = Vec::new();

    for (_, receiver) in request_receivers.streams() {
        let handle = spawn_local(handle_request_stream(
            config.clone(),
            torrents.clone(),
            response_senders.clone(),
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
    response_senders: Rc<Senders<ChannelResponse>>,
    mut stream: S,
) where
    S: Stream<Item = ChannelRequest> + ::std::marker::Unpin,
{
    let mut rng = SmallRng::from_entropy();

    let max_peer_age = config.cleaning.max_peer_age;
    let peer_valid_until = Rc::new(RefCell::new(ValidUntil::new(max_peer_age)));

    TimerActionRepeat::repeat(enclose!((peer_valid_until) move || {
        enclose!((peer_valid_until) move || async move {
            *peer_valid_until.borrow_mut() = ValidUntil::new(max_peer_age);

            Some(Duration::from_secs(1))
        })()
    }));

    while let Some(channel_request) = stream.next().await {
        let (response, consumer_id) = match channel_request {
            ChannelRequest::Announce {
                request,
                peer_addr,
                response_consumer_id,
                connection_id
            } => {
                let meta = ConnectionMeta {
                    worker_index: response_consumer_id.0,
                    poll_token: connection_id.0,
                    peer_addr,
                };

                let response = handle_announce_request(
                    &config,
                    &mut rng,
                    &mut torrents.borrow_mut(),
                    peer_valid_until.borrow().to_owned(),
                    meta,
                    request,
                );

                let response = ChannelResponse::Announce {
                    response,
                    peer_addr,
                    connection_id,
                };

                (response, response_consumer_id)
            }
            ChannelRequest::Scrape {
                request,
                peer_addr,
                response_consumer_id,
                connection_id,
                original_indices,
            } => {
                let meta = ConnectionMeta {
                    worker_index: response_consumer_id.0,
                    poll_token: connection_id.0,
                    peer_addr,
                };

                let response = handle_scrape_request(&config, &mut torrents.borrow_mut(), meta, request);

                let response = ChannelResponse::Scrape {
                    response,
                    peer_addr,
                    connection_id,
                    original_indices,
                };

                (response, response_consumer_id)
            }
        };

        ::log::debug!("preparing to send response to channel: {:?}", response);

        if let Err(err) = response_senders.try_send_to(consumer_id.0, response) {
            ::log::warn!("response_sender.try_send: {:?}", err);
        }

        yield_if_needed().await;
    }
}

