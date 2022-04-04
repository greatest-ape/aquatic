use std::net::IpAddr;

use aquatic_common::CanonicalSocketAddr;
use aquatic_http_protocol::{request::AnnounceRequest, response::FailureResponse};
use sqlx::{Executor, MySql, Pool};

#[derive(Debug)]
pub struct ValidatedAnnounceRequest(AnnounceRequest);

impl Into<AnnounceRequest> for ValidatedAnnounceRequest {
    fn into(self) -> AnnounceRequest {
        self.0
    }
}

#[derive(Debug, sqlx::FromRow)]
struct AnnounceProcedureResults {
    announce_allowed: bool,
    failure_reason: Option<String>,
    warning_message: Option<String>,
}

pub async fn validate_announce_request(
    pool: &Pool<MySql>,
    source_addr: CanonicalSocketAddr,
    user_agent: Option<String>,
    user_token: String,
    request: AnnounceRequest,
) -> Result<(ValidatedAnnounceRequest, Option<String>), FailureResponse> {
    match call_announce_procedure(pool, source_addr, user_agent, user_token, &request).await {
        Ok(results) => {
            if results.announce_allowed {
                Ok((ValidatedAnnounceRequest(request), results.warning_message))
            } else {
                Err(FailureResponse::new(
                    results
                        .failure_reason
                        .unwrap_or_else(|| "Not allowed".into()),
                ))
            }
        }
        Err(err) => {
            ::log::error!("announce procedure error: {:#}", err);

            Err(FailureResponse::new("Internal error"))
        }
    }
}

async fn call_announce_procedure(
    pool: &Pool<MySql>,
    source_addr: CanonicalSocketAddr,
    user_agent: Option<String>,
    user_token: String, // FIXME: length
    request: &AnnounceRequest,
) -> anyhow::Result<AnnounceProcedureResults> {
    let source_addr = source_addr.get();
    let source_ip_bytes: Vec<u8> = match source_addr.ip() {
        IpAddr::V4(ip) => ip.octets().into(),
        IpAddr::V6(ip) => ip.octets().into(),
    };

    let mut t = pool.begin().await?;

    t.execute(
        "
        SET
            @p_announce_allowed = false,
            @p_failure_reason = NULL,
            @p_warning_message = NULL;
        ",
    )
    .await?;

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
            ?,
            @p_announce_allowed,
            @p_failure_reason,
            @p_warning_message
        );
        ",
    )
    .bind(source_ip_bytes)
    .bind(source_addr.port())
    .bind(user_agent)
    .bind(user_token)
    .bind(hex::encode(request.info_hash.0))
    .bind(&request.peer_id.0[..])
    .bind(request.event.as_str())
    .bind(request.bytes_uploaded as u64)
    .bind(request.bytes_downloaded as u64)
    .bind(request.bytes_left as u64);

    t.execute(q).await?;

    let response = sqlx::query_as::<_, AnnounceProcedureResults>(
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
