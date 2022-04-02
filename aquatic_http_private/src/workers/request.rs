use tokio::sync::mpsc::Receiver;

use aquatic_http_protocol::response::{FailureResponse, Response};

use crate::common::ChannelAnnounceRequest;
use crate::config::Config;

pub fn run_request_worker(
    config: Config,
    request_receiver: Receiver<ChannelAnnounceRequest>,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_inner(config, request_receiver))?;

    Ok(())
}

async fn run_inner(
    config: Config,
    mut request_receiver: Receiver<ChannelAnnounceRequest>,
) -> anyhow::Result<()> {
    loop {
        let request = request_receiver
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("request channel closed"))?;

        println!("{:?}", request);

        let _ = request
            .response_sender
            .send(Response::Failure(FailureResponse::new(
                "successful actually",
            )));
    }
}
