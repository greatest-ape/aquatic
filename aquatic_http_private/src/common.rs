use tokio::sync::{mpsc, oneshot};

use aquatic_common::CanonicalSocketAddr;
use aquatic_http_protocol::{common::InfoHash, response::Response};

use crate::{config::Config, workers::socket::db::ValidatedAnnounceRequest};

#[derive(Debug)]
pub struct ChannelAnnounceRequest {
    pub request: ValidatedAnnounceRequest,
    pub source_addr: CanonicalSocketAddr,
    pub response_sender: oneshot::Sender<Response>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RequestWorkerIndex(pub usize);

impl RequestWorkerIndex {
    pub fn from_info_hash(config: &Config, info_hash: InfoHash) -> Self {
        Self(info_hash.0[0] as usize % config.swarm_workers)
    }
}

pub struct ChannelRequestSender(Vec<mpsc::Sender<ChannelAnnounceRequest>>);

impl ChannelRequestSender {
    pub fn new(senders: Vec<mpsc::Sender<ChannelAnnounceRequest>>) -> Self {
        Self(senders)
    }

    pub async fn send_to(
        &self,
        index: RequestWorkerIndex,
        request: ValidatedAnnounceRequest,
        source_addr: CanonicalSocketAddr,
    ) -> anyhow::Result<oneshot::Receiver<Response>> {
        let (response_sender, response_receiver) = oneshot::channel();

        let request = ChannelAnnounceRequest {
            request,
            source_addr,
            response_sender,
        };

        match self.0[index.0].send(request).await {
            Ok(()) => Ok(response_receiver),
            Err(err) => {
                Err(anyhow::Error::new(err).context("error sending ChannelAnnounceRequest"))
            }
        }
    }
}
