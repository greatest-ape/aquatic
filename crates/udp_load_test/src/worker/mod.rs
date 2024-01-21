use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mio::{net::UdpSocket, Events, Interest, Poll, Token};
use rand::Rng;
use rand::{prelude::SmallRng, SeedableRng};
use rand_distr::{Distribution, Gamma, WeightedIndex};
use socket2::{Domain, Protocol, Socket, Type};

use aquatic_udp_protocol::*;

use crate::config::Config;
use crate::{common::*, utils::*};

const MAX_PACKET_SIZE: usize = 8192;

pub struct Worker {
    config: Config,
    shared_state: LoadTestState,
    gamma: Gamma<f64>,
    addr: SocketAddr,
    socket: UdpSocket,
    buffer: [u8; MAX_PACKET_SIZE],
    rng: SmallRng,
    torrent_peers: TorrentPeerMap,
    statistics: SocketWorkerLocalStatistics,
}

impl Worker {
    pub fn run(shared_state: LoadTestState, gamma: Gamma<f64>, config: Config, addr: SocketAddr) {
        let socket = UdpSocket::from_std(create_socket(&config, addr));
        let buffer = [0u8; MAX_PACKET_SIZE];
        let rng = SmallRng::seed_from_u64(0xc3aa8be617b3acce);
        let torrent_peers = TorrentPeerMap::default();
        let statistics = SocketWorkerLocalStatistics::default();

        let mut instance = Self {
            config,
            shared_state,
            gamma,
            addr,
            socket,
            buffer,
            rng,
            torrent_peers,
            statistics,
        };

        instance.run_inner();
    }

    fn run_inner(&mut self) {
        let mut poll = Poll::new().expect("create poll");
        let mut events = Events::with_capacity(1);

        poll.registry()
            .register(&mut self.socket, Token(0), Interest::READABLE)
            .unwrap();

        // Bootstrap request cycle
        let initial_request = create_connect_request(generate_transaction_id(&mut self.rng));
        self.send_request(initial_request);

        let timeout = Duration::from_micros(self.config.network.poll_timeout);

        loop {
            poll.poll(&mut events, Some(timeout))
                .expect("failed polling");

            for _ in events.iter() {
                while let Ok(amt) = self.socket.recv(&mut self.buffer) {
                    match Response::from_bytes(&self.buffer[0..amt], self.addr.is_ipv4()) {
                        Ok(response) => {
                            if let Some(request) = self.process_response(response) {
                                self.send_request(request);
                            }
                        }
                        Err(err) => {
                            eprintln!("Received invalid response: {:#?}", err);
                        }
                    }
                }

                if self.rng.gen::<f32>() <= self.config.requests.additional_request_probability {
                    let additional_request =
                        create_connect_request(generate_transaction_id(&mut self.rng));

                    self.send_request(additional_request);
                }

                self.update_shared_statistics();
            }
        }
    }

    fn process_response(&mut self, response: Response) -> Option<Request> {
        match response {
            Response::Connect(r) => {
                self.statistics.responses_connect += 1;

                // Fetch the torrent peer or create it if is doesn't exists. Update
                // the connection id if fetched. Create a request and move the
                // torrent peer appropriately.

                let mut torrent_peer = self
                    .torrent_peers
                    .remove(&r.transaction_id)
                    .unwrap_or_else(|| self.create_torrent_peer(r.connection_id));

                torrent_peer.connection_id = r.connection_id;

                let new_transaction_id = generate_transaction_id(&mut self.rng);
                let request = self.create_random_request(new_transaction_id, &torrent_peer);

                self.torrent_peers.insert(new_transaction_id, torrent_peer);

                Some(request)
            }
            Response::AnnounceIpv4(r) => {
                self.statistics.responses_announce += 1;
                self.statistics.response_peers += r.peers.len();

                self.if_torrent_peer_move_and_create_random_request(r.fixed.transaction_id)
            }
            Response::AnnounceIpv6(r) => {
                self.statistics.responses_announce += 1;
                self.statistics.response_peers += r.peers.len();

                self.if_torrent_peer_move_and_create_random_request(r.fixed.transaction_id)
            }
            Response::Scrape(r) => {
                self.statistics.responses_scrape += 1;

                self.if_torrent_peer_move_and_create_random_request(r.transaction_id)
            }
            Response::Error(r) => {
                self.statistics.responses_error += 1;

                if !r.message.to_lowercase().contains("connection") {
                    eprintln!(
                        "Received error response which didn't contain the word 'connection': {}",
                        r.message
                    );
                }

                if let Some(torrent_peer) = self.torrent_peers.remove(&r.transaction_id) {
                    let new_transaction_id = generate_transaction_id(&mut self.rng);

                    self.torrent_peers.insert(new_transaction_id, torrent_peer);

                    Some(create_connect_request(new_transaction_id))
                } else {
                    Some(create_connect_request(generate_transaction_id(
                        &mut self.rng,
                    )))
                }
            }
        }
    }

    fn if_torrent_peer_move_and_create_random_request(
        &mut self,
        transaction_id: TransactionId,
    ) -> Option<Request> {
        let torrent_peer = self.torrent_peers.remove(&transaction_id)?;

        let new_transaction_id = generate_transaction_id(&mut self.rng);

        let request = self.create_random_request(new_transaction_id, &torrent_peer);

        self.torrent_peers.insert(new_transaction_id, torrent_peer);

        Some(request)
    }

    fn create_torrent_peer(&mut self, connection_id: ConnectionId) -> TorrentPeer {
        let num_scrape_hashes = self
            .rng
            .gen_range(1..self.config.requests.scrape_max_torrents);

        let scrape_hash_indices = (0..num_scrape_hashes)
            .map(|_| self.random_info_hash_index())
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let info_hash_index = self.random_info_hash_index();

        TorrentPeer {
            info_hash: self.shared_state.info_hashes[info_hash_index],
            scrape_hash_indices,
            connection_id,
            peer_id: generate_peer_id(),
            port: Port(self.rng.gen::<u16>().into()),
        }
    }

    fn create_random_request(
        &mut self,
        transaction_id: TransactionId,
        torrent_peer: &TorrentPeer,
    ) -> Request {
        const ITEMS: [RequestType; 3] = [
            RequestType::Announce,
            RequestType::Connect,
            RequestType::Scrape,
        ];

        let weights = [
            self.config.requests.weight_announce as u32,
            self.config.requests.weight_connect as u32,
            self.config.requests.weight_scrape as u32,
        ];

        let dist = WeightedIndex::new(weights).expect("random request weighted index");

        match ITEMS[dist.sample(&mut self.rng)] {
            RequestType::Announce => self.create_announce_request(torrent_peer, transaction_id),
            RequestType::Connect => (ConnectRequest { transaction_id }).into(),
            RequestType::Scrape => self.create_scrape_request(torrent_peer, transaction_id),
        }
    }

    fn create_announce_request(
        &mut self,
        torrent_peer: &TorrentPeer,
        transaction_id: TransactionId,
    ) -> Request {
        let (event, bytes_left) = {
            if self
                .rng
                .gen_bool(self.config.requests.peer_seeder_probability)
            {
                (AnnounceEvent::Completed, NumberOfBytes(0.into()))
            } else {
                (AnnounceEvent::Started, NumberOfBytes(50.into()))
            }
        };

        (AnnounceRequest {
            connection_id: torrent_peer.connection_id,
            action_placeholder: Default::default(),
            transaction_id,
            info_hash: torrent_peer.info_hash,
            peer_id: torrent_peer.peer_id,
            bytes_downloaded: NumberOfBytes(50.into()),
            bytes_uploaded: NumberOfBytes(50.into()),
            bytes_left,
            event: event.into(),
            ip_address: Ipv4AddrBytes([0; 4]),
            key: PeerKey(0.into()),
            peers_wanted: NumberOfPeers(self.config.requests.announce_peers_wanted.into()),
            port: torrent_peer.port,
        })
        .into()
    }

    fn create_scrape_request(
        &self,
        torrent_peer: &TorrentPeer,
        transaction_id: TransactionId,
    ) -> Request {
        let indeces = &torrent_peer.scrape_hash_indices;

        let mut scape_hashes = Vec::with_capacity(indeces.len());

        for i in indeces.iter() {
            scape_hashes.push(self.shared_state.info_hashes[*i].to_owned())
        }

        (ScrapeRequest {
            connection_id: torrent_peer.connection_id,
            transaction_id,
            info_hashes: scape_hashes,
        })
        .into()
    }

    fn random_info_hash_index(&mut self) -> usize {
        gamma_usize(
            &mut self.rng,
            self.gamma,
            &self.config.requests.number_of_torrents - 1,
        )
    }

    fn send_request(&mut self, request: Request) {
        let mut cursor = Cursor::new(self.buffer);

        match request.write(&mut cursor) {
            Ok(()) => {
                let position = cursor.position() as usize;
                let inner = cursor.get_ref();

                match self.socket.send(&inner[..position]) {
                    Ok(_) => {
                        self.statistics.requests += 1;
                    }
                    Err(err) => {
                        eprintln!("Couldn't send packet: {:?}", err);
                    }
                }
            }
            Err(err) => {
                eprintln!("request_to_bytes err: {}", err);
            }
        }
    }

    fn update_shared_statistics(&mut self) {
        self.shared_state
            .statistics
            .requests
            .fetch_add(self.statistics.requests, Ordering::Relaxed);
        self.shared_state
            .statistics
            .responses_connect
            .fetch_add(self.statistics.responses_connect, Ordering::Relaxed);
        self.shared_state
            .statistics
            .responses_announce
            .fetch_add(self.statistics.responses_announce, Ordering::Relaxed);
        self.shared_state
            .statistics
            .responses_scrape
            .fetch_add(self.statistics.responses_scrape, Ordering::Relaxed);
        self.shared_state
            .statistics
            .responses_error
            .fetch_add(self.statistics.responses_error, Ordering::Relaxed);
        self.shared_state
            .statistics
            .response_peers
            .fetch_add(self.statistics.response_peers, Ordering::Relaxed);

        self.statistics = SocketWorkerLocalStatistics::default();
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
