use super::common::*;



#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub struct TorrentScrapeStatistics {
    pub seeders:   NumberOfPeers,
    pub completed: NumberOfDownloads,
    pub leechers:  NumberOfPeers
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ConnectResponse {
    pub connection_id:     ConnectionId,
    pub transaction_id:    TransactionId
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AnnounceResponse {
    pub transaction_id:    TransactionId,
    pub announce_interval: AnnounceInterval,
    pub leechers:          NumberOfPeers,
    pub seeders:           NumberOfPeers,
    pub peers:             Vec<ResponsePeer>
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ScrapeResponse {
    pub transaction_id:    TransactionId,
    pub torrent_stats:     Vec<TorrentScrapeStatistics>
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ErrorResponse {
    pub transaction_id:    TransactionId,
    pub message:           String
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Response {
    Connect(ConnectResponse),
    Announce(AnnounceResponse),
    Scrape(ScrapeResponse),
    Error(ErrorResponse),
}
