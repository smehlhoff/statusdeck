mod analytics;
mod bookmarks;
mod catalog;
mod channels;
mod comments;
mod dashboard;
mod delivery_resend;
mod diagnostics;
mod health;
mod incidents;
mod monitors;
mod my_comments;
mod notification_summaries;
mod oidc;
mod profile;
mod reliability;
mod request;
mod rules;
mod session;
mod shared;
mod system_summary;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Router,
    http::StatusCode,
    middleware,
    routing::{get, patch},
};
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

use analytics::analytics;
use catalog::{catalog_components, catalog_provider, catalog_providers};
use channels::{channel_create, channel_delete, channel_test, channel_update, channels};
use comments::{comment_create, comment_delete, comment_update, comments};
use dashboard::dashboard;
use diagnostics::{data_metrics, deliveries, delivery_attempts, poll_runs, sources};
use health::{live, ready};
use incidents::{incident, incidents};
use monitors::{monitor_create, monitor_delete, monitor_update, monitors};
pub use request::{ApiError, Problem};
use request::{request_context, validate_request};
use rules::{rule_create, rule_delete, rule_update, rules};
use session::{csrf, session_create, session_delete, session_get};
use system_summary::system_summary;

use crate::{config::Config, db::Database};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub database: Database,
    started_at: Instant,
    oidc: Arc<crate::auth::oidc::RuntimeCache>,
    login_attempts: Arc<tokio::sync::Mutex<HashMap<String, Vec<Instant>>>>,
    password_verifications: Arc<tokio::sync::Semaphore>,
}

pub async fn router(config: Config, database: Database) -> anyhow::Result<Router> {
    const MAX_CONCURRENT_PASSWORD_VERIFICATIONS: usize = 2;

    let state = AppState {
        oidc: Arc::new(crate::auth::oidc::RuntimeCache::default()),
        started_at: Instant::now(),
        config,
        database,
        login_attempts: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        password_verifications: Arc::new(tokio::sync::Semaphore::new(
            MAX_CONCURRENT_PASSWORD_VERIFICATIONS,
        )),
    };
    Ok(Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/api/v1/csrf", get(csrf))
        .route("/api/v1/auth/methods", get(oidc::methods))
        .route("/api/v1/auth/oidc/start", axum::routing::post(oidc::start))
        .route("/api/v1/auth/oidc/callback", get(oidc::callback))
        .route(
            "/api/v1/auth/oidc/backchannel-logout",
            axum::routing::post(oidc::backchannel).layer(RequestBodyLimitLayer::new(20 * 1024)),
        )
        .route(
            "/api/v1/profile/oidc",
            get(oidc::status)
                .put(oidc::update_settings)
                .delete(oidc::clear_settings),
        )
        .route(
            "/api/v1/profile/oidc/link",
            axum::routing::post(oidc::link).delete(oidc::disconnect),
        )
        .route(
            "/api/v1/profile/oidc/link/confirm",
            axum::routing::post(oidc::confirm),
        )
        .route(
            "/api/v1/profile",
            get(profile::profile).patch(profile::profile_update),
        )
        .route("/api/v1/profile/email", patch(profile::email_update))
        .route("/api/v1/profile/password", patch(profile::password_update))
        .route(
            "/api/v1/profile/sessions",
            get(profile::sessions).delete(profile::sessions_revoke_others),
        )
        .route(
            "/api/v1/profile/security-activity",
            get(profile::security_activity),
        )
        .route(
            "/api/v1/profile/sessions/{id}",
            axum::routing::delete(profile::session_revoke),
        )
        .route(
            "/api/v1/session",
            get(session_get).post(session_create).delete(session_delete),
        )
        .route("/api/v1/catalog/providers", get(catalog_providers))
        .route("/api/v1/catalog/components", get(catalog_components))
        .route("/api/v1/catalog/providers/{id}", get(catalog_provider))
        .route(
            "/api/v1/catalog/providers/{id}/reliability",
            get(reliability::reliability),
        )
        .route("/api/v1/monitors", get(monitors).post(monitor_create))
        .route(
            "/api/v1/monitors/{id}",
            patch(monitor_update).delete(monitor_delete),
        )
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/analytics", get(analytics))
        .route("/api/v1/incidents", get(incidents))
        .route("/api/v1/incidents/{id}", get(incident))
        .route("/api/v1/bookmarks", get(bookmarks::bookmarks))
        .route("/api/v1/my-comments", get(my_comments::my_comments))
        .route(
            "/api/v1/incidents/{id}/bookmark",
            axum::routing::put(bookmarks::bookmark_save).delete(bookmarks::bookmark_delete),
        )
        .route(
            "/api/v1/incidents/{id}/comments",
            get(comments).post(comment_create),
        )
        .route(
            "/api/v1/incidents/{id}/comments/{comment_id}",
            patch(comment_update).delete(comment_delete),
        )
        .route(
            "/api/v1/notification-channels",
            get(channels).post(channel_create),
        )
        .route(
            "/api/v1/notification-channels/{id}",
            patch(channel_update).delete(channel_delete),
        )
        .route(
            "/api/v1/notification-channels/{id}/test",
            axum::routing::post(channel_test),
        )
        .route(
            "/api/v1/quiet-hours/preview",
            axum::routing::post(rules::quiet_hours_preview),
        )
        .route("/api/v1/alert-rules", get(rules).post(rule_create))
        .route(
            "/api/v1/alert-rules/{id}",
            patch(rule_update).delete(rule_delete),
        )
        .route("/api/v1/system/sources", get(sources))
        .route("/api/v1/system/summary", get(system_summary))
        .route("/api/v1/system/data-metrics", get(data_metrics))
        .route("/api/v1/system/poll-runs", get(poll_runs))
        .route("/api/v1/system/deliveries", get(deliveries))
        .route(
            "/api/v1/system/deliveries/{id}/resend",
            axum::routing::post(delivery_resend::resend),
        )
        .route(
            "/api/v1/notifications/summaries/{id}",
            get(notification_summaries::summary),
        )
        .route(
            "/api/v1/system/deliveries/{id}/attempts",
            get(delivery_attempts),
        )
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(RequestBodyLimitLayer::new(256 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            validate_request,
        ))
        .layer(middleware::from_fn(request_context))
        .with_state(state))
}
