use std::{str::FromStr, time::Instant};

use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header, uri::Authority},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tracing::Instrument;
use uuid::Uuid;

use super::AppState;

#[derive(Debug, Serialize)]
pub struct Problem {
    #[serde(rename = "type")]
    pub problem_type: &'static str,
    pub title: &'static str,
    pub status: u16,
    pub code: &'static str,
    pub detail: String,
    pub correlation_id: String,
}

tokio::task_local! {
    static REQUEST_ID: String;
}

pub(super) fn correlation_id() -> String {
    REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| Uuid::new_v4().to_string())
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    detail: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code,
            detail: detail.into(),
        }
    }

    pub fn unauthorized(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "authentication_required", detail)
    }

    pub fn forbidden(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", detail)
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(error = %error, "request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "The request could not be completed.",
        )
    }

    pub(super) fn bad_request(code: &'static str, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, detail)
    }

    pub(super) fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", detail)
    }

    pub(super) fn conflict(code: &'static str, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, detail)
    }

    pub(super) fn too_many_requests(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", detail)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(Problem {
            problem_type: "about:blank",
            title: self.status.canonical_reason().unwrap_or("Error"),
            status: self.status.as_u16(),
            code: self.code,
            detail: self.detail,
            correlation_id: REQUEST_ID
                .try_with(Clone::clone)
                .unwrap_or_else(|_| Uuid::new_v4().to_string()),
        });
        (self.status, body).into_response()
    }
}

pub(super) async fn request_context(request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .unwrap_or_else(Uuid::new_v4)
        .to_string();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %path,
        application_version = env!("CARGO_PKG_VERSION")
    );
    let started = Instant::now();
    REQUEST_ID
        .scope(request_id.clone(), async move {
            let mut response = next.run(request).instrument(span.clone()).await;
            if path == "/api/v1/auth/oidc/callback" {
                if !response.status().is_redirection() {
                    response = axum::response::Redirect::to("/login?oidc=failed").into_response();
                }
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response.headers_mut().insert(
                    header::REFERRER_POLICY,
                    HeaderValue::from_static("no-referrer"),
                );
            }
            if let Ok(value) = HeaderValue::from_str(&request_id) {
                response.headers_mut().insert("x-request-id", value);
            }
            tracing::info!(
                parent: &span,
                status = response.status().as_u16(),
                duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "request completed"
            );
            response
        })
        .await
}

pub(super) async fn validate_request(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let api_request = request.uri().path().starts_with("/api/");
    let forwarded_host = state
        .config
        .trust_proxy
        .then(|| request.headers().get("x-forwarded-host"))
        .flatten();
    let host = forwarded_host
        .or_else(|| request.headers().get(header::HOST))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Authority::from_str(value).ok())
        .map(|authority| authority.host().to_owned());
    if !state.config.trusted_hosts.is_empty()
        && !host.as_ref().is_some_and(|host| {
            state
                .config
                .trusted_hosts
                .iter()
                .any(|trusted| trusted.eq_ignore_ascii_case(host))
        })
    {
        return Err(ApiError::bad_request(
            "untrusted_host",
            "The request host is not allowed.",
        ));
    }
    let backchannel = request.method() == Method::POST
        && request.uri().path() == "/api/v1/auth/oidc/backchannel-logout";
    if !backchannel
        && !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        )
        && let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
    {
        let valid = url::Url::parse(origin).is_ok_and(|origin| {
            origin.scheme() == state.config.base_url.scheme()
                && origin.host_str() == state.config.base_url.host_str()
                && origin.port_or_known_default() == state.config.base_url.port_or_known_default()
        });
        if !valid {
            return Err(ApiError::forbidden("The request origin is not allowed."));
        }
    }
    let has_body = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > 0)
        || request.headers().contains_key(header::TRANSFER_ENCODING);
    if has_body
        && !backchannel
        && !request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                let media_type = value.split(';').next().unwrap_or_default().trim();
                media_type == "application/json" || media_type.ends_with("+json")
            })
    {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Request bodies must use a JSON content type.",
        ));
    }
    let response = next.run(request).await;
    let unstructured = !response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if unstructured
        && matches!(
            response.status(),
            StatusCode::BAD_REQUEST
                | StatusCode::UNPROCESSABLE_ENTITY
                | StatusCode::UNSUPPORTED_MEDIA_TYPE
        )
    {
        return Err(ApiError::new(
            response.status(),
            "invalid_request",
            "The request path or JSON body is invalid.",
        ));
    }
    if unstructured && api_request && response.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found(
            "The requested API resource was not found.",
        ));
    }
    if unstructured && response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "The request body exceeds the configured limit.",
        ));
    }
    if unstructured && response.status() == StatusCode::REQUEST_TIMEOUT {
        return Err(ApiError::new(
            StatusCode::REQUEST_TIMEOUT,
            "request_timeout",
            "The request exceeded the configured deadline.",
        ));
    }
    Ok(response)
}
