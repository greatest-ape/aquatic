use aquatic_common::CanonicalSocketAddr;
use aquatic_http_protocol::response::Response;
use tokio::sync::oneshot::Sender;

use super::socket::db::ValidatedAnnounceRequest;

#[derive(Debug)]
pub struct ChannelAnnounceRequest {
    pub request: ValidatedAnnounceRequest,
    pub source_addr: CanonicalSocketAddr,
    pub response_sender: Sender<Response>,
}
