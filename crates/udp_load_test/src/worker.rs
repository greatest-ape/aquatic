use std::io::{Cursor, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::Ordering;
use std::time::Duration;

use aquatic_common::IndexMap;
use crossbeam_channel::Sender;
use rand::Rng;
use rand::{prelude::SmallRng, SeedableRng};
use rand_distr::{Distribution, WeightedIndex};
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_udp_protocol::*;

use crate::common::{LoadTestState, Peer};
use crate::config::Config;
use crate::StatisticsMessage;

const MAX_PACKET_SIZE: usize = 8192;

pub struct Worker {
    config: Config,
    shared_state: LoadTestState,
    peers: Box<[Peer]>,
    request_type_dist: RequestTypeDist,
    addr: SocketAddr,
    sockets: Vec<UdpSocket>,
    buffer: [u8; MAX_PACKET_SIZE],
    rng: SmallRng,
    statistics: LocalStatistics,
    statistics_sender: Sender<StatisticsMessage>,
    announce_responses_per_info_hash: IndexMap<usize, u64>,
}

impl Worker {
    pub fn run(
        config: Config,
        shared_state: LoadTestState,
        statistics_sender: Sender<StatisticsMessage>,
        peers: Box<[Peer]>,
        addr: SocketAddr,
    ) {
        let mut sockets = Vec::new();

        for _ in 0..config.network.sockets_per_worker {
            sockets.push(create_socket(&config, addr));
        }

        let buffer = [0u8; MAX_PACKET_SIZE];
        let rng = SmallRng::seed_from_u64(0xc3aa8be617b3acce);
        let statistics = LocalStatistics::default();
        let request_type_dist = RequestTypeDist::new(&config).unwrap();

        let mut instance = Self {
            config,
            shared_state,
            peers,
            request_type_dist,
            addr,
            sockets,
            buffer,
            rng,
            statistics,
            statistics_sender,
            announce_responses_per_info_hash: Default::default(),
        };

        instance.run_inner();
    }

    fn run_inner(&mut self) {
        let mut connection_ids = Vec::new();

        for _ in 0..self.config.network.sockets_per_worker {
            connection_ids.push(self.acquire_connection_id());
        }

        let mut requests_sent = 0usize;
        let mut responses_received = 0usize;

        let mut connect_socket_index = 0u8;
        let mut peer_index = 0usize;
        let mut loop_index = 0usize;

        loop {
            let response_ratio = responses_received as f64 / requests_sent.max(1) as f64;

            if response_ratio >= 0.90 || requests_sent == 0 || self.rng.gen::<u8>() == 0 {
                for _ in 0..self.sockets.len() {
                    match self.request_type_dist.sample(&mut self.rng) {
                        RequestType::Connect => {
                            self.send_connect_request(
                                connect_socket_index,
                                connect_socket_index.into(),
                            );

                            connect_socket_index = connect_socket_index.wrapping_add(1)
                                % self.config.network.sockets_per_worker;
                        }
                        RequestType::Announce => {
                            self.send_announce_request(&connection_ids, peer_index);

                            peer_index = (peer_index + 1) % self.peers.len();
                        }
                        RequestType::Scrape => {
                            self.send_scrape_request(&connection_ids, peer_index);

                            peer_index = (peer_index + 1) % self.peers.len();
                        }
                    }

                    requests_sent += 1;
                }
            }

            for socket_index in 0..self.sockets.len() {
                // Do this instead of iterating over Vec to fix borrow checker complaint
                let socket = self.sockets.get(socket_index).unwrap();

                match socket.recv(&mut self.buffer[..]) {
                    Ok(amt) => {
                        match Response::parse_bytes(&self.buffer[0..amt], self.addr.is_ipv4()) {
                            Ok(Response::Connect(r)) => {
                                // If we're sending connect requests, we might
                                // as well keep connection IDs valid
                                let connection_id_index =
                                    u32::from_ne_bytes(r.transaction_id.0.get().to_ne_bytes())
                                        as usize;
                                connection_ids[connection_id_index] = r.connection_id;

                                self.handle_response(Response::Connect(r));
                            }
                            Ok(response) => {
                                self.handle_response(response);
                            }
                            Err(err) => {
                                eprintln!("Received invalid response: {:#?}", err);
                            }
                        }

                        responses_received += 1;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => (),
                    Err(err) => {
                        eprintln!("recv error: {:#}", err);
                    }
                }
            }

            if loop_index % 1024 == 0 {
                self.update_shared_statistics();
            }

            loop_index = loop_index.wrapping_add(1);
        }
    }

    fn acquire_connection_id(&mut self) -> ConnectionId {
        loop {
            self.send_connect_request(0, u32::MAX);

            for _ in 0..100 {
                match self.sockets[0].recv(&mut self.buffer[..]) {
                    Ok(amt) => {
                        match Response::parse_bytes(&self.buffer[0..amt], self.addr.is_ipv4()) {
                            Ok(Response::Connect(r)) => {
                                return r.connection_id;
                            }
                            Ok(r) => {
                                eprintln!("Received non-connect response: {:?}", r);
                            }
                            Err(err) => {
                                eprintln!("Received invalid response: {:#?}", err);
                            }
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        ::std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(err) => {
                        eprintln!("recv error: {:#}", err);
                    }
                };
            }
        }
    }

    fn send_connect_request(&mut self, socket_index: u8, transaction_id: u32) {
        let transaction_id = TransactionId::new(i32::from_ne_bytes(transaction_id.to_ne_bytes()));

        let request = ConnectRequest { transaction_id };

        let mut cursor = Cursor::new(self.buffer);

        request.write_bytes(&mut cursor).unwrap();

        let position = cursor.position() as usize;

        match self.sockets[socket_index as usize].send(&cursor.get_ref()[..position]) {
            Ok(_) => {
                self.statistics.requests += 1;
            }
            Err(err) => {
                eprintln!("Couldn't send packet: {:?}", err);
            }
        }
    }

    fn send_announce_request(&mut self, connection_ids: &[ConnectionId], peer_index: usize) {
        let peer = self.peers.get(peer_index).unwrap();

        let (event, bytes_left) = {
            if self
                .rng
                .gen_bool(self.config.requests.peer_seeder_probability)
            {
                (AnnounceEvent::Completed, NumberOfBytes::new(0))
            } else {
                (AnnounceEvent::Started, NumberOfBytes::new(50))
            }
        };

        let transaction_id =
            TransactionId::new(i32::from_ne_bytes((peer_index as u32).to_ne_bytes()));

        let request = AnnounceRequest {
            connection_id: connection_ids[peer.socket_index as usize],
            action_placeholder: Default::default(),
            transaction_id,
            info_hash: peer.announce_info_hash,
            peer_id: PeerId([0; 20]),
            bytes_downloaded: NumberOfBytes::new(50),
            bytes_uploaded: NumberOfBytes::new(50),
            bytes_left,
            event: event.into(),
            ip_address: Ipv4AddrBytes([0; 4]),
            key: PeerKey::new(0),
            peers_wanted: NumberOfPeers::new(self.config.requests.announce_peers_wanted),
            port: peer.announce_port,
        };

        let mut cursor = Cursor::new(self.buffer);

        request.write_bytes(&mut cursor).unwrap();

        let position = cursor.position() as usize;

        match self.sockets[peer.socket_index as usize].send(&cursor.get_ref()[..position]) {
            Ok(_) => {
                self.statistics.requests += 1;
            }
            Err(err) => {
                eprintln!("Couldn't send packet: {:?}", err);
            }
        }
    }

    fn send_scrape_request(&mut self, connection_ids: &[ConnectionId], peer_index: usize) {
        let peer = self.peers.get(peer_index).unwrap();

        let transaction_id =
            TransactionId::new(i32::from_ne_bytes((peer_index as u32).to_ne_bytes()));

        let mut info_hashes = Vec::with_capacity(peer.scrape_info_hash_indices.len());

        for i in peer.scrape_info_hash_indices.iter() {
            info_hashes.push(self.shared_state.info_hashes[*i].to_owned())
        }

        let request = ScrapeRequest {
            connection_id: connection_ids[peer.socket_index as usize],
            transaction_id,
            info_hashes,
        };

        let mut cursor = Cursor::new(self.buffer);

        request.write_bytes(&mut cursor).unwrap();

        let position = cursor.position() as usize;

        match self.sockets[peer.socket_index as usize].send(&cursor.get_ref()[..position]) {
            Ok(_) => {
                self.statistics.requests += 1;
            }
            Err(err) => {
                eprintln!("Couldn't send packet: {:?}", err);
            }
        }
    }

    fn handle_response(&mut self, response: Response) {
        match response {
            Response::Connect(_) => {
                self.statistics.responses_connect += 1;
            }
            Response::AnnounceIpv4(r) => {
                self.statistics.responses_announce += 1;
                self.statistics.response_peers += r.peers.len();

                let peer_index =
                    u32::from_ne_bytes(r.fixed.transaction_id.0.get().to_ne_bytes()) as usize;

                if let Some(peer) = self.peers.get(peer_index) {
                    *self
                        .announce_responses_per_info_hash
                        .entry(peer.announce_info_hash_index)
                        .or_default() += 1;
                }
            }
            Response::AnnounceIpv6(r) => {
                self.statistics.responses_announce += 1;
                self.statistics.response_peers += r.peers.len();

                let peer_index =
                    u32::from_ne_bytes(r.fixed.transaction_id.0.get().to_ne_bytes()) as usize;

                if let Some(peer) = self.peers.get(peer_index) {
                    *self
                        .announce_responses_per_info_hash
                        .entry(peer.announce_info_hash_index)
                        .or_default() += 1;
                }
            }
            Response::Scrape(_) => {
                self.statistics.responses_scrape += 1;
            }
            Response::Error(_) => {
                self.statistics.responses_error += 1;
            }
        }
    }

    fn update_shared_statistics(&mut self) {
        let shared_statistics = &self.shared_state.statistics;

        shared_statistics
            .requests
            .fetch_add(self.statistics.requests, Ordering::Relaxed);
        shared_statistics
            .responses_connect
            .fetch_add(self.statistics.responses_connect, Ordering::Relaxed);
        shared_statistics
            .responses_announce
            .fetch_add(self.statistics.responses_announce, Ordering::Relaxed);
        shared_statistics
            .responses_scrape
            .fetch_add(self.statistics.responses_scrape, Ordering::Relaxed);
        shared_statistics
            .responses_error
            .fetch_add(self.statistics.responses_error, Ordering::Relaxed);
        shared_statistics
            .response_peers
            .fetch_add(self.statistics.response_peers, Ordering::Relaxed);

        if self.config.extra_statistics {
            let message = StatisticsMessage::ResponsesPerInfoHash(
                self.announce_responses_per_info_hash.split_off(0),
            );

            self.statistics_sender.try_send(message).unwrap();
        }

        self.statistics = LocalStatistics::default();
    }
}

fn create_socket(config: &Config, addr: SocketAddr) -> ::std::net::UdpSocket {
    let socket = if addr.is_ipv4() {
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
    } else {
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
    }
    .expect("create socket");

    socket
        .set_nonblocking(true)
        .expect("socket: set nonblocking");

    if config.network.recv_buffer != 0 {
        if let Err(err) = socket.set_recv_buffer_size(config.network.recv_buffer) {
            eprintln!(
                "socket: failed setting recv buffer to {}: {:?}",
                config.network.recv_buffer, err
            );
        }
    }

    socket
        .bind(&addr.into())
        .unwrap_or_else(|err| panic!("socket: bind to {}: {:?}", addr, err));

    socket
        .connect(&config.server_address.into())
        .expect("socket: connect to server");

    socket.into()
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum RequestType {
    Announce,
    Connect,
    Scrape,
}

pub struct RequestTypeDist(WeightedIndex<usize>);

impl RequestTypeDist {
    fn new(config: &Config) -> anyhow::Result<Self> {
        let weights = [
            config.requests.weight_announce,
            config.requests.weight_connect,
            config.requests.weight_scrape,
        ];

        Ok(Self(WeightedIndex::new(weights)?))
    }

    fn sample(&self, rng: &mut impl Rng) -> RequestType {
        const ITEMS: [RequestType; 3] = [
            RequestType::Announce,
            RequestType::Connect,
            RequestType::Scrape,
        ];

        ITEMS[self.0.sample(rng)]
    }
}

#[derive(Default)]
pub struct LocalStatistics {
    pub requests: usize,
    pub response_peers: usize,
    pub responses_connect: usize,
    pub responses_announce: usize,
    pub responses_scrape: usize,
    pub responses_error: usize,
}
