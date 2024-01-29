use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;

use aquatic_http_protocol::request::Request;

static INPUT: &[u8] = b"GET /announce?info_hash=%04%0bkV%3f%5cr%14%a6%b7%98%adC%c3%c9.%40%24%00%b9&peer_id=-TR2940-5ert69muw5t8&port=11000&uploaded=0&downloaded=0&left=0&numwant=0&key=3ab4b977&compact=1&supportcrypto=1&event=stopped HTTP/1.1\r\n\r\n";

pub fn bench(c: &mut Criterion) {
    c.bench_function("request-from-bytes", |b| {
        b.iter(|| Request::parse_bytes(black_box(INPUT)))
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
