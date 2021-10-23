use std::cell::RefCell;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::time::Duration;

use futures_lite::{Stream, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::timer::TimerActionRepeat;
use glommio::{enclose, prelude::*};
use rand::prelude::SmallRng;
use rand::SeedableRng;

use crate::common::announce::handle_announce_request;
use crate::common::*;
use crate::config::Config;
use crate::glommio::common::update_access_list;

pub async fn run_request_worker(
    config: Config,
    request_mesh_builder: MeshBuilder<(usize, AnnounceRequest, SocketAddr), Partial>,
    response_mesh_builder: MeshBuilder<(AnnounceResponse, SocketAddr), Partial>,
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
            update_access_list(config.clone(), access_list.clone()).await;

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
    response_senders: Rc<Senders<(AnnounceResponse, SocketAddr)>>,
    mut stream: S,
) where
    S: Stream<Item = (usize, AnnounceRequest, SocketAddr)> + ::std::marker::Unpin,
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

    while let Some((producer_index, request, addr)) = stream.next().await {
        let response = match addr.ip() {
            IpAddr::V4(ip) => handle_announce_request(
                &config,
                &mut rng,
                &mut torrents.borrow_mut().ipv4,
                request,
                ip,
                peer_valid_until.borrow().to_owned(),
            ),
            IpAddr::V6(ip) => handle_announce_request(
                &config,
                &mut rng,
                &mut torrents.borrow_mut().ipv6,
                request,
                ip,
                peer_valid_until.borrow().to_owned(),
            ),
        };

        ::log::debug!("preparing to send response to channel: {:?}", response);

        if let Err(err) = response_senders.try_send_to(producer_index, (response, addr)) {
            ::log::warn!("response_sender.try_send: {:?}", err);
        }

        yield_if_needed().await;
    }
}
