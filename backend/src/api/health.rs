use std::time::Duration;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use super::AppState;

pub(super) async fn live() -> Json<Value> {
    Json(json!({"status": "alive"}))
}

pub(super) async fn ready(State(state): State<AppState>) -> Response {
    let checks = tokio::time::timeout(Duration::from_secs(2), async {
        let database = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.database.pool);
        let poller = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM worker_heartbeats WHERE role = 'poller' AND heartbeat_at > now() - interval '2 minutes')").fetch_one(&state.database.pool);
        let dispatcher = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM worker_heartbeats WHERE role = 'dispatcher' AND heartbeat_at > now() - interval '2 minutes')").fetch_one(&state.database.pool);
        let migrations = state.database.migrations_current();
        let (database, poller, dispatcher, migrations) = tokio::join!(database, poller, dispatcher, migrations);
        (database.is_ok(), poller.unwrap_or(false), dispatcher.unwrap_or(false), migrations)
    }).await;
    let (db_ok, poller_ok, dispatcher_ok, migrations_ok) = match checks {
        Ok(checks) => checks,
        Err(_) => {
            tracing::warn!("readiness checks exceeded the two-second deadline");
            (false, false, false, false)
        }
    };
    let ready = db_ok && migrations_ok && poller_ok && dispatcher_ok;
    if !ready {
        tracing::warn!(
            database = db_ok,
            migrations = migrations_ok,
            poller = poller_ok,
            dispatcher = dispatcher_ok,
            "readiness check failed"
        );
    }
    let body = Json(json!({"status": if ready {"ready"} else {"not_ready"}}));
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        body,
    )
        .into_response()
}
