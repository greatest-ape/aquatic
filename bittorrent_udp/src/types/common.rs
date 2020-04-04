use std::net;



#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum IpVersion {
    IPv4,
    IPv6
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct AnnounceInterval (pub i32);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct InfoHash (pub [u8; 20]);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct ConnectionId (pub i64);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct TransactionId (pub i32);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfBytes (pub i64);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfPeers (pub i32);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct NumberOfDownloads (pub i32);


#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct Port (pub u16);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug, PartialOrd, Ord)]
pub struct PeerId (pub [u8; 20]);

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct PeerKey (pub u32);


#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct ResponsePeer {
    pub ip_address:    net::IpAddr,
    pub port:          Port,
}