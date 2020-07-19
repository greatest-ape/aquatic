use std::time::Duration;
use std::vec::Drain;

use crossbeam_channel::{Receiver, Sender};
use rand::distributions::WeightedIndex;
use rand::prelude::*;

use crate::common::*;
use crate::utils::*;


pub fn run_handler_thread(
    config: &Config,
    state: LoadTestState,
    request_senders: Vec<Sender<Request>>,
    response_receiver: Receiver<(ThreadId, Response)>,
){
    let state = &state;

    let mut rng1 = SmallRng::from_rng(thread_rng())
        .expect("create SmallRng from thread_rng()");
    let mut rng2 = SmallRng::from_rng(thread_rng())
        .expect("create SmallRng from thread_rng()");
    
    let timeout = Duration::from_micros(config.handler.channel_timeout);

    let mut responses = Vec::new();

    loop {
        for i in 0..config.handler.max_responses_per_iter {
            let response = if i == 0 {
                match response_receiver.recv(){
                    Ok(r) => r,
                    Err(_) => break, // Really shouldn't happen
                }
            } else {
                match response_receiver.recv_timeout(timeout){
                    Ok(r) => r,
                    Err(_) => break,
                }
            };

            responses.push(response);
        }

        let requests = process_responses(
            config,
            &state,
            &mut rng1,
            responses.drain(..)
        );

        // Somewhat dubious heuristic for deciding how fast to create
        // and send additional requests (requests not having anything
        // to do with previously sent requests)
        let num_additional_to_send = {
            let num_additional_requests = requests.iter()
                .map(|v| v.len())
                .sum::<usize>() as f64;
            
            let num_new_requests_per_socket = num_additional_requests /
                config.num_socket_workers as f64;

            ((num_new_requests_per_socket / 1.2) * config.handler.additional_request_factor) as usize + 10
        };

        for (channel_index, new_requests) in requests.into_iter().enumerate(){
            let channel = &request_senders[channel_index];

            for _ in 0..num_additional_to_send {
                let request = create_random_request(
                    &config,
                    &state,
                    &mut rng2,
                );

                channel.send(request)
                    .expect("send request to channel in handler worker");
            }
            
            for request in new_requests.into_iter(){
                channel.send(request)
                    .expect("send request to channel in handler worker");
            }
        }
    }
}


fn process_responses(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
    responses: Drain<(ThreadId, Response)>
) -> Vec<Vec<Request>> {
    let mut new_requests = Vec::with_capacity(
        config.num_socket_workers as usize
    );

    for _ in 0..config.num_socket_workers {
        new_requests.push(Vec::new());
    }

    for (socket_thread_id, response) in responses.into_iter() {
        let new_request = create_random_request(config, state, rng);

        new_requests[socket_thread_id.0 as usize].push(new_request);
    }

    new_requests
}


pub fn create_random_request(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
) -> Request {
    let weights = vec![
        config.handler.weight_announce as u32,
        config.handler.weight_scrape as u32, 
    ];

    let items = vec![
        RequestType::Announce,
        RequestType::Scrape,
    ];

    let dist = WeightedIndex::new(&weights)
        .expect("random request weighted index");

    match items[dist.sample(rng)] {
        RequestType::Announce => create_announce_request(
            config,
            state,
            rng,
        ),
        RequestType::Scrape => create_scrape_request(
            config,
            state,
            rng,
        )
    }
}


fn create_announce_request(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
) -> Request {
    let (event, bytes_left) = {
        if rng.gen_bool(config.handler.peer_seeder_probability) {
            (AnnounceEvent::Completed, 0)
        } else {
            (AnnounceEvent::Started, 50)
        }
    };

    let info_hash_index = select_info_hash_index(config, &state, rng);

    Request::Announce(AnnounceRequest {
        info_hash: state.info_hashes[info_hash_index],
        peer_id: PeerId(rng.gen()),
        bytes_left,
        event,
        key: None,
        numwant: None,
        compact: true,
        port: rng.gen()
    })
}


fn create_scrape_request(
    config: &Config,
    state: &LoadTestState,
    rng: &mut impl Rng,
) -> Request {
    let mut scrape_hashes = Vec::new();

    for _ in 0..20 {
        let info_hash_index = select_info_hash_index(config, &state, rng);

        scrape_hashes.push(state.info_hashes[info_hash_index]);
    }

    Request::Scrape(ScrapeRequest {
        info_hashes: scrape_hashes,
    })
}