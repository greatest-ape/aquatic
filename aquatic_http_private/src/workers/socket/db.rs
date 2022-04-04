use std::net::IpAddr;

use aquatic_common::CanonicalSocketAddr;
use aquatic_http_protocol::{
    common::{AnnounceEvent, PeerId}, request::AnnounceRequest, response::FailureResponse,
};
use sqlx::{Executor, MySql, Pool};

#[derive(Debug)]
pub struct ValidatedAnnounceRequest(AnnounceRequest);

impl Into<AnnounceRequest> for ValidatedAnnounceRequest {
    fn into(self) -> AnnounceRequest {
        self.0
    }
}

#[derive(Debug)]
struct AnnounceProcedureParameters {
    source_ip: IpAddr,
    source_port: u16,
    user_agent: Option<String>,
    user_token: String,
    info_hash: String,
    peer_id: PeerId,
    event: AnnounceEvent,
    uploaded: u64,
    downloaded: u64,
    left: u64,
}

impl AnnounceProcedureParameters {
    fn new(
        source_addr: CanonicalSocketAddr,
        user_agent: Option<String>,
        user_token: String, // FIXME: length
        request: &AnnounceRequest,
    ) -> Self {
        Self {
            source_ip: source_addr.get().ip(),
            source_port: source_addr.get().port(),
            user_agent,
            user_token,
            info_hash: hex::encode(request.info_hash.0),
            peer_id: request.peer_id,
            event: request.event,
            uploaded: request.bytes_uploaded as u64,
            downloaded: request.bytes_downloaded as u64,
            left: request.bytes_left as u64,
        }
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
    let parameters =
        AnnounceProcedureParameters::new(source_addr, user_agent, user_token, &request);

    match call_announce_procedure(pool, parameters).await {
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
    parameters: AnnounceProcedureParameters,
) -> anyhow::Result<AnnounceProcedureResults> {
    let source_ip_bytes: Vec<u8> = match parameters.source_ip {
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
            ?,
            @p_announce_allowed,
            @p_failure_reason,
            @p_warning_message
        );
        ",
    )
    .bind(source_ip_bytes)
    .bind(parameters.source_port)
    .bind(parameters.user_agent)
    .bind(parameters.user_token)
    .bind(parameters.info_hash)
    .bind(&parameters.peer_id.0[..])
    .bind(parameters.event.as_str())
    .bind(parameters.uploaded)
    .bind(parameters.downloaded)
    .bind(parameters.left);

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
