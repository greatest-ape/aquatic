use std::net::{IpAddr, Ipv4Addr};

use sqlx::{Executor, MySql, Pool};

pub struct DbAnnounceRequest {
    source_ip: IpAddr,
    source_port: u16,
    user_agent: Option<String>,
    user_token: String,
    info_hash: String,
    peer_id: String,
    event: String,
    uploaded: u64,
    downloaded: u64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct DbAnnounceResponse {
    pub announce_allowed: bool,
    pub failure_reason: Option<String>,
    pub warning_message: Option<String>,
}

pub async fn announce(pool: &Pool<MySql>) -> Result<DbAnnounceResponse, sqlx::Error> {
    let request = DbAnnounceRequest {
        source_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        source_port: 1000,
        user_agent: Some("rtorrent".into()),
        user_token: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        info_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        peer_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        event: "started".into(),
        uploaded: 50,
        downloaded: 100,
    };

    let announce_response = get_announce_response(&pool, request).await?;

    Ok(announce_response)
}

async fn get_announce_response(
    pool: &Pool<MySql>,
    request: DbAnnounceRequest,
) -> Result<DbAnnounceResponse, sqlx::Error> {
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
    .bind(request.event)
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
    .await;

    t.commit().await?;

    response
}
