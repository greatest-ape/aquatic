use axum::{
    extract::{ConnectInfo, Path, RawQuery},
    headers::UserAgent,
    http::StatusCode,
    Extension, TypedHeader,
};
use sqlx::mysql::MySqlPool;
use std::net::SocketAddr;

use aquatic_http_protocol::request::AnnounceRequest;

use super::db;

pub async fn announce(
    Extension(pool): Extension<MySqlPool>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    opt_user_agent: Option<TypedHeader<UserAgent>>,
    Path(user_token): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<String, (StatusCode, String)> {
    let request = AnnounceRequest::from_query_string(&query.unwrap_or_else(|| "".into()))
        .map_err(anyhow_error)?;

    let opt_user_agent = opt_user_agent.map(|header| header.as_str().to_owned());

    let db_announce_request =
        db::DbAnnounceRequest::new(peer_addr, opt_user_agent, user_token, request);

    let db_announce_result = db::get_announce_response(&pool, db_announce_request)
        .await
        .map_err(anyhow_error)?;

    Ok(format!("{:?}", db_announce_result))
}

fn anyhow_error(err: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
