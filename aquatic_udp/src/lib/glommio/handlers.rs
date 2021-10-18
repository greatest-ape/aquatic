use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use glommio::prelude::*;
use glommio::channels::shared_channel::{SharedReceiver, SharedSender};
use rand::SeedableRng;
use rand::prelude::SmallRng;

use crate::config::Config;
use crate::common::*;
use crate::common::announce::handle_announce_request;

pub fn run_request_worker(
    config: Config,
    request_receiver: SharedReceiver<(AnnounceRequest, SocketAddr)>,
    response_sender: SharedSender<(AnnounceResponse, SocketAddr)>,
) {
    LocalExecutorBuilder::default()
        .spawn(|| async move {
            let request_receiver = request_receiver.connect().await;
            let response_sender = response_sender.connect().await;

            let mut rng = SmallRng::from_entropy();

            // Need to be cleaned periodically: use timer?
            let mut torrents_ipv4 = TorrentMap::<Ipv4Addr>::default();
            let mut torrents_ipv6 = TorrentMap::<Ipv6Addr>::default();

            // Needs to be updated periodically: use timer?
            let peer_valid_until = ValidUntil::new(config.cleaning.max_peer_age);

            while let Some((request, addr)) = request_receiver.recv().await {
                let response = match addr.ip() {
                    IpAddr::V4(ip) => {
                        handle_announce_request(&config, &mut rng, &mut torrents_ipv4, request, ip, peer_valid_until)
                    },
                    IpAddr::V6(ip) => {
                        handle_announce_request(&config, &mut rng, &mut torrents_ipv6, request, ip, peer_valid_until)
                    },
                };

                if let Err(err) = response_sender.try_send((response, addr)) {
                    ::log::warn!("response_sender.try_send: {:?}", err);
                }
            }

        })
        .expect("failed to spawn local executor")
        .join()
        .unwrap();
}