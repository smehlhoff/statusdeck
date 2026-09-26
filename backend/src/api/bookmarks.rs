use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{
        authenticated_user, decode_page_cursor, encode_page_cursor, require_csrf, validated_search,
    },
};

const PAGE_SIZE: i64 = 20;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BookmarksQuery {
    q: Option<String>,
    provider_id: Option<Uuid>,
    cursor: Option<String>,
}

#[derive(Serialize, FromRow)]
pub(super) struct BookmarkRow {
    id: Uuid,
    title: String,
    kind: String,
    severity: String,
    lifecycle: String,
    providers: Value,
    bookmarked_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct BookmarkProvider {
    id: Uuid,
    name: String,
}

#[derive(Serialize)]
pub(super) struct BookmarkPage {
    items: Vec<BookmarkRow>,
    next_cursor: Option<String>,
    providers: Vec<BookmarkProvider>,
}

pub(super) async fn bookmarks(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<BookmarksQuery>,
) -> Result<Json<BookmarkPage>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    let search = validated_search(query.q.as_deref())?;
    let (cursor_time, cursor_id) = query
        .cursor
        .as_deref()
        .map(decode_page_cursor)
        .transpose()?
        .unzip();
    // Bookmarks remain visible even when a provider is no longer monitored or active.
    let mut items = sqlx::query_as::<_, BookmarkRow>(
        "SELECT i.id, i.title, i.kind, i.severity, i.lifecycle, b.created_at AS bookmarked_at,
                COALESCE((SELECT jsonb_agg(jsonb_build_object('id', p.id, 'name', p.name) ORDER BY p.name, p.id)
                    FROM incident_providers link JOIN providers p ON p.id = link.provider_id
                    WHERE link.incident_id = i.id), '[]'::jsonb) AS providers
         FROM incident_bookmarks b JOIN incidents i ON i.id = b.incident_id
         WHERE b.user_id = $1
           AND ($2::text IS NULL OR i.title ILIKE '%' || $2 || '%' OR i.upstream_incident_id ILIKE '%' || $2 || '%')
           AND ($3::uuid IS NULL OR EXISTS (SELECT 1 FROM incident_providers link WHERE link.incident_id = i.id AND link.provider_id = $3))
           AND ($4::timestamptz IS NULL OR (b.created_at, b.incident_id) < ($4, $5))
         ORDER BY b.created_at DESC, b.incident_id DESC LIMIT $6",
    ).bind(user.id).bind(search).bind(query.provider_id).bind(cursor_time).bind(cursor_id)
        .bind(PAGE_SIZE + 1).fetch_all(&state.database.pool).await.map_err(ApiError::internal)?;
    let has_more = i64::try_from(items.len()).is_ok_and(|len| len > PAGE_SIZE);
    if has_more {
        items.pop();
    }
    let next_cursor = has_more
        .then(|| {
            items
                .last()
                .map(|row| encode_page_cursor(row.bookmarked_at, row.id))
        })
        .flatten();
    let providers = sqlx::query_as::<_, BookmarkProvider>(
        "SELECT DISTINCT p.id, p.name FROM incident_bookmarks b
         JOIN incident_providers link ON link.incident_id = b.incident_id
         JOIN providers p ON p.id = link.provider_id
         WHERE b.user_id = $1 ORDER BY p.name, p.id",
    )
    .bind(user.id)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(BookmarkPage {
        items,
        next_cursor,
        providers,
    }))
}

pub(super) async fn bookmark_save(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let exists = sqlx::query_scalar::<_, bool>(
        "WITH saved AS (
            INSERT INTO incident_bookmarks (user_id, incident_id)
            SELECT $1, id FROM incidents WHERE id = $2
            ON CONFLICT DO NOTHING
         ) SELECT EXISTS (SELECT 1 FROM incidents WHERE id = $2)",
    )
    .bind(user.id)
    .bind(id)
    .fetch_one(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    if !exists {
        return Err(ApiError::not_found("incident not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn bookmark_delete(
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    sqlx::query("DELETE FROM incident_bookmarks WHERE user_id = $1 AND incident_id = $2")
        .bind(user.id)
        .bind(id)
        .execute(&state.database.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}
