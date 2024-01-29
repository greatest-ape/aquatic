use std::net::Ipv4Addr;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use aquatic_http_protocol::response::*;

pub fn bench(c: &mut Criterion) {
    let mut peers = Vec::new();

    for i in 0..100 {
        peers.push(ResponsePeer {
            ip_address: Ipv4Addr::new(127, 0, 0, i),
            port: i as u16,
        })
    }

    let announce_response = AnnounceResponse {
        announce_interval: 120,
        complete: 100,
        incomplete: 500,
        peers: ResponsePeerListV4(peers),
        peers6: ResponsePeerListV6(Vec::new()),
        warning_message: None,
    };

    let response = Response::Announce(announce_response);

    let mut buffer = [0u8; 4096];
    let mut buffer = ::std::io::Cursor::new(&mut buffer[..]);

    c.bench_function("announce-response-to-bytes", |b| {
        b.iter(|| {
            buffer.set_position(0);

            Response::write_bytes(black_box(&response), black_box(&mut buffer)).unwrap();
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(1000)
        .measurement_time(Duration::from_secs(180))
        .significance_level(0.01);
    targets = bench
}
criterion_main!(benches);
