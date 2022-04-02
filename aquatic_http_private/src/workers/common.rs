use aquatic_common::CanonicalSocketAddr;
use aquatic_http_protocol::response::AnnounceResponse;
use tokio::sync::oneshot::Sender;

use super::socket::db::ValidatedAnnounceRequest;

pub struct ChannelAnnounceRequest {
    request: ValidatedAnnounceRequest,
    source_addr: CanonicalSocketAddr,
    response_sender: Sender<AnnounceResponse>,
}