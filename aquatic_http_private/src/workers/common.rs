use aquatic_common::CanonicalSocketAddr;
use aquatic_http_protocol::response::AnnounceResponse;
use tokio::sync::oneshot::Sender;

use super::socket::db::ValidatedAnnounceRequest;

pub struct ChannelAnnounceRequest {
    pub request: ValidatedAnnounceRequest,
    pub source_addr: CanonicalSocketAddr,
    pub response_sender: Sender<AnnounceResponse>,
}
