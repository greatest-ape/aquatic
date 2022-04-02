use std::net::{IpAddr, SocketAddr};

use aquatic_http_protocol::{common::AnnounceEvent, request::AnnounceRequest};
use sqlx::{Executor, MySql, Pool};

#[derive(Debug)]
pub struct DbAnnounceRequest {
    source_ip: IpAddr,
    source_port: u16,
    user_agent: Option<String>,
    user_token: String,
    info_hash: String,
    peer_id: String,
    event: AnnounceEvent,
    uploaded: u64,
    downloaded: u64,
}

impl DbAnnounceRequest {
    pub fn new(
        source_addr: SocketAddr,
        user_agent: Option<String>,
        user_token: String, // FIXME: length
        request: AnnounceRequest,
    ) -> Self {
        Self {
            source_ip: source_addr.ip(),
            source_port: source_addr.port(),
            user_agent,
            user_token,
            info_hash: hex::encode(request.info_hash.0),
            peer_id: hex::encode(request.peer_id.0),
            event: request.event,
            uploaded: 0,   // FIXME
            downloaded: 0, // FIXME
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct DbAnnounceResponse {
    pub announce_allowed: bool,
    pub failure_reason: Option<String>,
    pub warning_message: Option<String>,
}

pub async fn get_announce_response(
    pool: &Pool<MySql>,
    request: DbAnnounceRequest,
) -> anyhow::Result<DbAnnounceResponse> {
    let source_ip_bytes: Vec<u8> = match request.source_ip {
        IpAddr::V4(ip) => ip.octets().into(),
        IpAddr::V6(ip) => ip.octets().into(),
    };

    let mut t = pool.begin().await?;

    t.execute("SET @p_announce_allowed = false;").await?;
    t.execute("SET @p_failure_reason = NULL;").await?;
    t.execute("SET @p_warning_message = NULL;").await?;

    let q = sqlx::query(
        "
        CALL aquatic_announce_v1(
            ?,
            ?,
            ?,
            ?,
            ?,
            ?,
            ?,
            ?,
            ?,
            @p_announce_allowed,
            @p_failure_reason,
            @p_warning_message
        );
        ",
    )
    .bind(source_ip_bytes)
    .bind(request.source_port)
    .bind(request.user_agent)
    .bind(request.user_token)
    .bind(request.info_hash)
    .bind(request.peer_id)
    .bind(request.event.as_str())
    .bind(request.uploaded)
    .bind(request.downloaded);

    t.execute(q).await?;

    let response = sqlx::query_as::<_, DbAnnounceResponse>(
        "
        SELECT
            @p_announce_allowed as announce_allowed,
            @p_failure_reason as failure_reason,
            @p_warning_message as warning_message;
        
        ",
    )
    .fetch_one(&mut t)
    .await?;

    t.commit().await?;

    Ok(response)
}
