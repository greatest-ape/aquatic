use axum::{
    extract::{Path, RawQuery},
    headers::UserAgent,
    http::StatusCode,
    Extension, TypedHeader,
};
use sqlx::mysql::MySqlPool;

use super::db;

pub async fn announce(
    Extension(pool): Extension<MySqlPool>,
    opt_user_agent: Option<TypedHeader<UserAgent>>,
    Path(user_token): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<String, (StatusCode, String)> {
    match db::announce(&pool).await {
        Ok(r) => Ok(format!("{:?}", r)),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}
