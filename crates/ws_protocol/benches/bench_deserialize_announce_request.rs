use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;

use aquatic_ws_protocol::{
    common::*,
    incoming::{AnnounceEvent, AnnounceRequest, AnnounceRequestOffer, InMessage},
};

pub fn bench(c: &mut Criterion) {
    let info_hash = InfoHash([
        b'a', b'b', b'c', b'd', b'e', b'?', b'\n', b'1', b'2', b'3', 0, 1, 2, 3, 4, 0, 1, 2, 3, 4,
    ]);
    let peer_id = PeerId(info_hash.0);
    let offers: Vec<AnnounceRequestOffer> = (0..10)
        .map(|i| {
            let mut offer_id = OfferId(info_hash.0);

            offer_id.0[i] = i as u8;

            AnnounceRequestOffer {
                offer: RtcOffer {
                    t: RtcOfferType::Offer,
                    sdp: "abcdef".into(),
                },
                offer_id,
            }
        })
        .collect();
    let offers_len = offers.len();

    let request = InMessage::AnnounceRequest(AnnounceRequest {
        action: AnnounceAction::Announce,
        info_hash,
        peer_id,
        bytes_left: Some(2),
        event: Some(AnnounceEvent::Started),
        offers: Some(offers),
        numwant: Some(offers_len),
        answer: Some(RtcAnswer {
            t: RtcAnswerType::Answer,
            sdp: "abcdef".into(),
        }),
        answer_to_peer_id: Some(peer_id),
        answer_offer_id: Some(OfferId(info_hash.0)),
    });

    let ws_message = request.to_ws_message();

    c.bench_function("deserialize-announce-request", |b| {
        b.iter(|| InMessage::from_ws_message(black_box(ws_message.clone())))
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
