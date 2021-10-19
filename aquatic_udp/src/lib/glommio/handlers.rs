use std::cell::RefCell;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::rc::Rc;

use futures_lite::{Stream, StreamExt};
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role, Senders};
use glommio::prelude::*;
use rand::prelude::SmallRng;
use rand::SeedableRng;

use crate::common::announce::handle_announce_request;
use crate::common::*;
use crate::config::Config;

pub async fn run_request_worker(
    config: Config,
    request_mesh_builder: MeshBuilder<(usize, AnnounceRequest, SocketAddr), Partial>,
    response_mesh_builder: MeshBuilder<(AnnounceResponse, SocketAddr), Partial>,
) {
    let (_, mut request_receivers) = request_mesh_builder.join(Role::Consumer).await.unwrap();
    let (response_senders, _) = response_mesh_builder.join(Role::Producer).await.unwrap();

    let response_senders = Rc::new(response_senders);

    // Need to be cleaned periodically: use timer?
    let torrents_ipv4 = Rc::new(RefCell::new(TorrentMap::<Ipv4Addr>::default()));
    let torrents_ipv6 = Rc::new(RefCell::new(TorrentMap::<Ipv6Addr>::default()));

    for (_, receiver) in request_receivers.streams() {
        handle_request_stream(
            &config,
            torrents_ipv4.clone(),
            torrents_ipv6.clone(),
            response_senders.clone(),
            receiver,
        )
        .await;
    }
}

async fn handle_request_stream<S>(
    config: &Config,
    torrents_ipv4: Rc<RefCell<TorrentMap<Ipv4Addr>>>,
    torrents_ipv6: Rc<RefCell<TorrentMap<Ipv6Addr>>>,
    response_senders: Rc<Senders<(AnnounceResponse, SocketAddr)>>,
    mut stream: S,
) where
    S: Stream<Item = (usize, AnnounceRequest, SocketAddr)> + ::std::marker::Unpin,
{
    let mut rng = SmallRng::from_entropy();

    // Needs to be updated periodically: use timer?
    let peer_valid_until = ValidUntil::new(config.cleaning.max_peer_age);

    while let Some((producer_index, request, addr)) = stream.next().await {
        let response = match addr.ip() {
            IpAddr::V4(ip) => handle_announce_request(
                &config,
                &mut rng,
                &mut torrents_ipv4.borrow_mut(),
                request,
                ip,
                peer_valid_until,
            ),
            IpAddr::V6(ip) => handle_announce_request(
                &config,
                &mut rng,
                &mut torrents_ipv6.borrow_mut(),
                request,
                ip,
                peer_valid_until,
            ),
        };

        if let Err(err) = response_senders.try_send_to(producer_index, (response, addr)) {
            ::log::warn!("response_sender.try_send: {:?}", err);
        }

        yield_if_needed().await;
    }
}
