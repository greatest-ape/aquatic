use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use futures_lite::stream::empty;
use futures_lite::StreamExt;
use glommio::channels::channel_mesh::{MeshBuilder, Partial, Role};
use glommio::prelude::*;
use rand::prelude::SmallRng;
use rand::SeedableRng;

use crate::common::announce::handle_announce_request;
use crate::common::*;
use crate::config::Config;

pub fn run_request_worker(
    config: Config,
    request_mesh_builder: MeshBuilder<(usize, AnnounceRequest, SocketAddr), Partial>,
    response_mesh_builder: MeshBuilder<(AnnounceResponse, SocketAddr), Partial>,
) {
    LocalExecutorBuilder::default()
        .spawn(|| async move {
            let (_, mut request_receivers) =
                request_mesh_builder.join(Role::Consumer).await.unwrap();
            let (response_senders, _) = response_mesh_builder.join(Role::Producer).await.unwrap();

            let mut rng = SmallRng::from_entropy();

            // Need to be cleaned periodically: use timer?
            let mut torrents_ipv4 = TorrentMap::<Ipv4Addr>::default();
            let mut torrents_ipv6 = TorrentMap::<Ipv6Addr>::default();

            // Needs to be updated periodically: use timer?
            let peer_valid_until = ValidUntil::new(config.cleaning.max_peer_age);

            let mut stream = empty().boxed_local();

            for (_, receiver) in request_receivers.streams() {
                stream = Box::pin(stream.race(receiver));
            }

            while let Some((producer_index, request, addr)) = stream.next().await {
                let response = match addr.ip() {
                    IpAddr::V4(ip) => handle_announce_request(
                        &config,
                        &mut rng,
                        &mut torrents_ipv4,
                        request,
                        ip,
                        peer_valid_until,
                    ),
                    IpAddr::V6(ip) => handle_announce_request(
                        &config,
                        &mut rng,
                        &mut torrents_ipv6,
                        request,
                        ip,
                        peer_valid_until,
                    ),
                };

                if let Err(err) = response_senders.try_send_to(producer_index, (response, addr)) {
                    ::log::warn!("response_sender.try_send: {:?}", err);
                }
            }
        })
        .expect("failed to spawn local executor")
        .join()
        .unwrap();
}
