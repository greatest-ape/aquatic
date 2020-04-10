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


impl From<ConnectResponse> for Response {
    fn from(r: ConnectResponse) -> Self {
        Self::Connect(r)
    }
}


impl From<AnnounceResponse> for Response {
    fn from(r: AnnounceResponse) -> Self {
        Self::Announce(r)
    }
}


impl From<ScrapeResponse> for Response {
    fn from(r: ScrapeResponse) -> Self {
        Self::Scrape(r)
    }
}


impl From<ErrorResponse> for Response {
    fn from(r: ErrorResponse) -> Self {
        Self::Error(r)
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for TorrentScrapeStatistics {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            seeders: NumberOfPeers(i32::arbitrary(g)),
            completed: NumberOfDownloads(i32::arbitrary(g)),
            leechers: NumberOfPeers(i32::arbitrary(g)),
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ConnectResponse {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        Self {
            connection_id: ConnectionId(i64::arbitrary(g)),
            transaction_id: TransactionId(i32::arbitrary(g)),
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for AnnounceResponse {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        let peers = (0..u8::arbitrary(g)).map(|_| {
            ResponsePeer::arbitrary(g)
        }).collect();

        Self {
            transaction_id: TransactionId(i32::arbitrary(g)),
            announce_interval: AnnounceInterval(i32::arbitrary(g)),
            leechers: NumberOfPeers(i32::arbitrary(g)),
            seeders: NumberOfPeers(i32::arbitrary(g)),
            peers,
        }
    }
}


#[cfg(test)]
impl quickcheck::Arbitrary for ScrapeResponse {
    fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
        let torrent_stats = (0..u8::arbitrary(g)).map(|_| {
            TorrentScrapeStatistics::arbitrary(g)
        }).collect();

        Self {
            transaction_id: TransactionId(i32::arbitrary(g)),
            torrent_stats,
        }
    }
}