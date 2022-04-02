use aquatic_common::CanonicalSocketAddr;
use axum::{
    extract::{ConnectInfo, Path, RawQuery},
    headers::UserAgent,
    http::StatusCode,
    response::IntoResponse,
    Extension, TypedHeader,
};
use sqlx::mysql::MySqlPool;
use std::net::SocketAddr;
use tokio::sync::oneshot;

use aquatic_http_protocol::{
    request::AnnounceRequest,
    response::{AnnounceResponse, FailureResponse, Response},
};

use crate::workers::common::ChannelAnnounceRequest;

use super::db;

pub async fn announce(
    Extension(pool): Extension<MySqlPool>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    opt_user_agent: Option<TypedHeader<UserAgent>>,
    Path(user_token): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<(StatusCode, impl IntoResponse), (StatusCode, impl IntoResponse)> {
    let request = AnnounceRequest::from_query_string(&query.unwrap_or_else(|| "".into()))
        .map_err(|err| build_response(Response::Failure(FailureResponse::new("Internal error"))))?;

    let opt_user_agent = opt_user_agent.map(|header| header.as_str().to_owned());

    let validated_request =
        db::validate_announce_request(&pool, peer_addr, opt_user_agent, user_token, request)
            .await
            .map_err(|r| build_response(Response::Failure(r)))?;

    let (response_sender, response_receiver) = oneshot::channel();

    let canonical_socket_addr = CanonicalSocketAddr::new(peer_addr);

    let channel_request = ChannelAnnounceRequest {
        request: validated_request,
        source_addr: canonical_socket_addr,
        response_sender,
    };

    // TODO: send request to request worker

    let response = response_receiver.await.map_err(|err| {
        ::log::error!("channel response sender closed: {}", err);

        build_response(Response::Failure(FailureResponse::new("Internal error")))
    })?;

    Ok(build_response(Response::Announce(response)))
}

fn build_response(response: Response) -> (StatusCode, impl IntoResponse) {
    let mut response_bytes = Vec::with_capacity(512);

    response.write(&mut response_bytes);

    (StatusCode::OK, response_bytes)
}
