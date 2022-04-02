use aquatic_common::CanonicalSocketAddr;
use axum::{
    extract::{ConnectInfo, Path, RawQuery},
    headers::UserAgent,
    http::StatusCode,
    response::IntoResponse,
    Extension, TypedHeader,
};
use sqlx::mysql::MySqlPool;
use std::{borrow::Cow, net::SocketAddr, sync::Arc};

use aquatic_http_protocol::{
    request::AnnounceRequest,
    response::{FailureResponse, Response},
};

use crate::{
    common::{ChannelRequestSender, RequestWorkerIndex},
    config::Config,
};

use super::db;

pub async fn announce(
    Extension(config): Extension<Arc<Config>>,
    Extension(pool): Extension<MySqlPool>,
    Extension(request_sender): Extension<Arc<ChannelRequestSender>>,
    ConnectInfo(source_addr): ConnectInfo<SocketAddr>,
    opt_user_agent: Option<TypedHeader<UserAgent>>,
    Path(user_token): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<axum::response::Response, axum::response::Response> {
    let query = query.ok_or_else(|| create_failure_response("Empty query string"))?;

    let request = AnnounceRequest::from_query_string(&query)
        .map_err(|_| create_failure_response("Malformed request"))?;

    let request_worker_index = RequestWorkerIndex::from_info_hash(&config, request.info_hash);
    let opt_user_agent = opt_user_agent.map(|header| header.as_str().to_owned());

    let source_addr = CanonicalSocketAddr::new(source_addr);

    let (validated_request, opt_warning_message) =
        db::validate_announce_request(&pool, source_addr, opt_user_agent, user_token, request)
            .await
            .map_err(|r| create_response(Response::Failure(r)))?;

    let response_receiver = request_sender
        .send_to(request_worker_index, validated_request, source_addr)
        .await
        .map_err(|err| internal_error(format!("Sending request over channel failed: {:#}", err)))?;

    let mut response = response_receiver.await.map_err(|err| {
        internal_error(format!("Receiving response over channel failed: {:#}", err))
    })?;

    if let Response::Announce(ref mut r) = response {
        r.warning_message = opt_warning_message;
    }

    Ok(create_response(response))
}

fn create_response(response: Response) -> axum::response::Response {
    let mut response_bytes = Vec::with_capacity(64);

    response.write(&mut response_bytes).unwrap();

    (
        StatusCode::OK,
        [("Content-type", "text/plain; charset=utf-8")],
        response_bytes,
    )
        .into_response()
}

fn create_failure_response<R: Into<Cow<'static, str>>>(reason: R) -> axum::response::Response {
    let mut response_bytes = Vec::with_capacity(32);

    FailureResponse::new(reason)
        .write(&mut response_bytes)
        .unwrap();

    (
        StatusCode::OK,
        [("Content-type", "text/plain; charset=utf-8")],
        response_bytes,
    )
        .into_response()
}

fn internal_error(error: String) -> axum::response::Response {
    ::log::error!("{}", error);

    create_failure_response("Internal error")
}
