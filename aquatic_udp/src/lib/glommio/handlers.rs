use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

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

use super::common::State;

pub async fn run_request_worker(
    config: Config,
    state: State,
    request_mesh_builder: MeshBuilder<(usize, ConnectedRequest, SocketAddr), Partial>,
    response_mesh_builder: MeshBuilder<(ConnectedResponse, SocketAddr), Partial>,
) {
    let (_, mut request_receivers) = request_mesh_builder.join(Role::Consumer).await.unwrap();
    let (response_senders, _) = response_mesh_builder.join(Role::Producer).await.unwrap();
    let response_senders = Rc::new(response_senders);

    let torrents = Rc::new(RefCell::new(TorrentMaps::default()));

    // Periodically clean torrents
    TimerActionRepeat::repeat(enclose!((config, torrents, state) move || {
        enclose!((config, torrents, state) move || async move {
            torrents.borrow_mut().clean(&config, &state.access_list);

            Some(Duration::from_secs(config.cleaning.torrent_cleaning_interval))
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
    response_senders: Rc<Senders<(ConnectedResponse, SocketAddr)>>,
    mut stream: S,
) where
    S: Stream<Item = (usize, ConnectedRequest, SocketAddr)> + ::std::marker::Unpin,
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

    while let Some((producer_index, request, src)) = stream.next().await {
        let response = match request {
            ConnectedRequest::Announce(request) => {
                ConnectedResponse::Announce(handle_announce_request(
                    &config,
                    &mut rng,
                    &mut torrents.borrow_mut(),
                    request,
                    src,
                    peer_valid_until.borrow().to_owned(),
                ))
            }
            ConnectedRequest::Scrape {
                request,
                original_indices,
            } => {
                let response = handle_scrape_request(&mut torrents.borrow_mut(), src, request);

                ConnectedResponse::Scrape {
                    response,
                    original_indices,
                }
            }
        };

        ::log::debug!("preparing to send response to channel: {:?}", response);

        if let Err(err) = response_senders
            .send_to(producer_index, (response, src))
            .await
        {
            ::log::error!("response_sender.send: {:?}", err);
        }

        yield_if_needed().await;
    }
}
