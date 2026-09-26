use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::{ApiError, AppState, shared::authenticated_user};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReliabilityQuery {
    days: Option<i64>,
    component_id: Option<Uuid>,
    time_zone: Option<String>,
}

pub(super) async fn reliability(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ReliabilityQuery>,
) -> Result<Json<Value>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let days = query.days.unwrap_or(365);
    if ![90, 180, 365].contains(&days) {
        return Err(ApiError::bad_request(
            "invalid_days",
            "Choose 90, 180, or 365 days.",
        ));
    }
    let source_id: Uuid =
        sqlx::query_scalar("SELECT provider_source_id FROM providers WHERE id = $1 AND active")
            .bind(id)
            .fetch_optional(&state.database.pool)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("provider not found"))?;
    if let Some(component_id) = query.component_id {
        let belongs: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM components WHERE id = $1 AND provider_id = $2)",
        )
        .bind(component_id)
        .bind(id)
        .fetch_one(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
        if !belongs {
            return Err(ApiError::bad_request(
                "invalid_component",
                "Component does not belong to this provider.",
            ));
        }
    }
    let time_zone = query.time_zone.as_deref().unwrap_or("UTC");
    if time_zone.len() > 100 {
        return Err(ApiError::bad_request(
            "invalid_time_zone",
            "Choose a valid time zone.",
        ));
    }
    let time_zone = time_zone
        .parse::<Tz>()
        .map_err(|_| ApiError::bad_request("invalid_time_zone", "Choose a valid time zone."))?;
    let to = Utc::now();
    let first_day = to.with_timezone(&time_zone).date_naive() - Duration::days(days - 1);
    // Match the daily buckets' handling of skipped or repeated local midnights.
    let from: DateTime<Utc> = sqlx::query_scalar("SELECT $1::date::timestamp AT TIME ZONE $2")
        .bind(first_day)
        .bind(time_zone.name())
        .fetch_one(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    let result: sqlx::types::Json<Value> = sqlx::query_scalar(include_str!("reliability.sql"))
        .bind(id)
        .bind(from)
        .bind(to)
        .bind(query.component_id)
        .bind(source_id)
        .bind(time_zone.name())
        .fetch_one(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(result.0))
}
