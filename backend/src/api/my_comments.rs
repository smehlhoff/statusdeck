use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, decode_page_cursor, encode_page_cursor, validated_search},
};

const PAGE_SIZE: i64 = 20;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MyCommentsQuery {
    q: Option<String>,
    provider_id: Option<Uuid>,
    cursor: Option<String>,
}

#[derive(Serialize, FromRow)]
pub(super) struct MyCommentRow {
    id: Uuid,
    incident_id: Uuid,
    title: String,
    body: String,
    kind: String,
    severity: String,
    lifecycle: String,
    providers: Value,
    created_at: DateTime<Utc>,
    edited_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, FromRow)]
struct CommentProvider {
    id: Uuid,
    name: String,
}

#[derive(Serialize)]
pub(super) struct MyCommentPage {
    items: Vec<MyCommentRow>,
    next_cursor: Option<String>,
    providers: Vec<CommentProvider>,
}

pub(super) async fn my_comments(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<MyCommentsQuery>,
) -> Result<Json<MyCommentPage>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    let search = validated_search(query.q.as_deref())?;
    let (cursor_time, cursor_id) = query
        .cursor
        .as_deref()
        .map(decode_page_cursor)
        .transpose()?
        .unzip();
    // Personal comments remain visible after unmonitoring or catalog removal.
    let mut items = sqlx::query_as::<_, MyCommentRow>(
        "SELECT c.id, i.id AS incident_id, i.title, c.body, i.kind, i.severity, i.lifecycle, c.created_at, c.edited_at,
                COALESCE((SELECT jsonb_agg(jsonb_build_object('id', p.id, 'name', p.name) ORDER BY p.name, p.id)
                    FROM incident_providers link JOIN providers p ON p.id = link.provider_id
                    WHERE link.incident_id = i.id), '[]'::jsonb) AS providers
         FROM incident_comments c JOIN incidents i ON i.id = c.incident_id
         WHERE c.author_user_id = $1
           AND ($2::text IS NULL OR i.title ILIKE '%' || $2 || '%' OR i.upstream_incident_id ILIKE '%' || $2 || '%' OR c.body ILIKE '%' || $2 || '%')
           AND ($3::uuid IS NULL OR EXISTS (SELECT 1 FROM incident_providers link WHERE link.incident_id = i.id AND link.provider_id = $3))
           AND ($4::timestamptz IS NULL OR (c.created_at, c.id) < ($4, $5))
         ORDER BY c.created_at DESC, c.id DESC LIMIT $6",
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
                .map(|row| encode_page_cursor(row.created_at, row.id))
        })
        .flatten();
    let providers = sqlx::query_as::<_, CommentProvider>(
        "SELECT DISTINCT p.id, p.name FROM incident_comments c
         JOIN incident_providers link ON link.incident_id = c.incident_id
         JOIN providers p ON p.id = link.provider_id
         WHERE c.author_user_id = $1 ORDER BY p.name, p.id",
    )
    .bind(user.id)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(MyCommentPage {
        items,
        next_cursor,
        providers,
    }))
}
