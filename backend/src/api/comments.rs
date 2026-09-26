use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, decode_page_cursor, encode_page_cursor, require_csrf},
};

const MAX_COMMENT_CHARACTERS: usize = 5_000;
const COMMENT_PAGE_SIZE: i64 = 50;

#[derive(Serialize, FromRow)]
pub(super) struct CommentRow {
    id: Uuid,
    author_user_id: Uuid,
    author_email: String,
    author_display_name: String,
    body: String,
    created_at: DateTime<Utc>,
    edited_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommentsQuery {
    cursor: Option<String>,
}

#[derive(Serialize)]
pub(super) struct CommentPage {
    items: Vec<CommentRow>,
    next_cursor: Option<String>,
    total_count: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommentRequest {
    body: String,
}

fn validated_body(body: &str) -> Result<&str, ApiError> {
    let body = body.trim();
    if body.is_empty() || body.chars().count() > MAX_COMMENT_CHARACTERS || body.contains('\0') {
        return Err(ApiError::bad_request(
            "invalid_comment",
            "Comments must contain between 1 and 5,000 characters and cannot contain null characters.",
        ));
    }
    Ok(body)
}

pub(super) async fn comments(
    headers: HeaderMap,
    Path(incident_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<CommentsQuery>,
) -> Result<Json<CommentPage>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    let cursor = query
        .cursor
        .as_deref()
        .map(decode_page_cursor)
        .transpose()?;
    let (cursor_time, cursor_id) = cursor.unzip();
    let total_count = sqlx::query_scalar::<_, i64>(
        "SELECT (SELECT count(*) FROM incident_comments WHERE incident_id = $1) FROM incidents WHERE id = $1",
    )
    .bind(incident_id)
    .fetch_optional(&state.database.pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("incident not found"))?;
    let mut items = sqlx::query_as::<_, CommentRow>(
        "SELECT comment.id, comment.author_user_id, author.email AS author_email, author.display_name AS author_display_name, comment.body, comment.created_at, comment.edited_at
         FROM incident_comments comment JOIN users author ON author.id = comment.author_user_id
         WHERE comment.incident_id = $1 AND ($2::timestamptz IS NULL OR (comment.created_at, comment.id) < ($2, $3))
         ORDER BY comment.created_at DESC, comment.id DESC LIMIT $4",
    )
    .bind(incident_id)
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(COMMENT_PAGE_SIZE + 1)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;
    let has_more = i64::try_from(items.len()).is_ok_and(|len| len > COMMENT_PAGE_SIZE);
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
    Ok(Json(CommentPage {
        items,
        next_cursor,
        total_count,
    }))
}

pub(super) async fn comment_create(
    headers: HeaderMap,
    Path(incident_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(input): Json<CommentRequest>,
) -> Result<(StatusCode, Json<CommentRow>), ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let body = validated_body(&input.body)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let row = sqlx::query_as::<_, CommentRow>(
        "INSERT INTO incident_comments (incident_id, author_user_id, body)
         SELECT id, $2, $3 FROM incidents WHERE id = $1
         RETURNING id, author_user_id, $4::text AS author_email, (SELECT display_name FROM users WHERE users.id = author_user_id) AS author_display_name, body, created_at, edited_at",
    )
    .bind(incident_id)
    .bind(user.id)
    .bind(body)
    .bind(&user.email)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("incident not found"))?;
    audit_comment(&mut tx, user.id, "comment.created", incident_id, row.id).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub(super) async fn comment_update(
    headers: HeaderMap,
    Path((incident_id, comment_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    Json(input): Json<CommentRequest>,
) -> Result<Json<CommentRow>, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let body = validated_body(&input.body)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let row = sqlx::query_as::<_, CommentRow>(
        "UPDATE incident_comments SET body = $3, edited_at = CASE WHEN body IS DISTINCT FROM $3 THEN now() ELSE edited_at END
         WHERE id = $2 AND incident_id = $1
         RETURNING id, author_user_id, (SELECT email FROM users WHERE users.id = author_user_id) AS author_email, (SELECT display_name FROM users WHERE users.id = author_user_id) AS author_display_name, body, created_at, edited_at",
    )
    .bind(incident_id)
    .bind(comment_id)
    .bind(body)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::not_found("comment not found"))?;
    audit_comment(&mut tx, user.id, "comment.updated", incident_id, comment_id).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(row))
}

pub(super) async fn comment_delete(
    headers: HeaderMap,
    Path((incident_id, comment_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated_user(&state, &headers).await?;
    require_csrf(&headers)?;
    let mut tx = state
        .database
        .pool
        .begin()
        .await
        .map_err(ApiError::internal)?;
    let result = sqlx::query("DELETE FROM incident_comments WHERE id = $2 AND incident_id = $1")
        .bind(incident_id)
        .bind(comment_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("comment not found"));
    }
    audit_comment(&mut tx, user.id, "comment.deleted", incident_id, comment_id).await?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn audit_comment(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    action: &str,
    incident_id: Uuid,
    comment_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id, metadata) VALUES ($1, $2, 'incident_comment', $3, jsonb_build_object('incident_id', $4::uuid))")
        .bind(actor_id)
        .bind(action)
        .bind(comment_id)
        .bind(incident_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}
