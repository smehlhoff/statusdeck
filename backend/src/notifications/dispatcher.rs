use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    config::Config,
    notifications::{is_valid_channel_target, secrets},
};

pub(crate) const DELIVERY_LEASE_SECONDS: i64 = 120;
const ZULIP_TITLE_LIMIT: usize = 256;
// ponytail: use Zulip's default ceiling; query server settings if custom limits are needed.
const ZULIP_MESSAGE_LIMIT: usize = 10_000;
const NTFY_TITLE_BYTES: usize = 1024;
const NTFY_MESSAGE_BYTES: usize = 4096;
const PUSH_MESSAGE_CHARACTERS: usize = 1900;
// Conservative presentation budgets for our single Slack/Mattermost attachment.
const CHAT_TITLE_LIMIT: usize = 256;
const CHAT_FIELD_LIMIT: usize = 1024;
const CHAT_UPDATE_LIMIT: usize = 4000;

type HmacSha256 = Hmac<Sha256>;

struct Delivery {
    id: Uuid,
    event_id: Uuid,
    event_type: String,
    entity_id: Uuid,
    lifecycle_generation: i32,
    event_created_at: DateTime<Utc>,
    channel_id: Uuid,
    channel_type: String,
    encrypted_config: String,
    payload: Value,
}

struct DeliveryResponse {
    status: StatusCode,
    retry_after_seconds: Option<u64>,
}

pub async fn dispatch_once(pool: &PgPool, config: &Config, owner: &str) -> Result<bool> {
    let Some(delivery) = claim(pool, owner).await? else {
        return Ok(false);
    };
    tracing::info!(delivery_id = %delivery.id, notification_event_id = %delivery.event_id, channel_id = %delivery.channel_id, worker_instance_id = %owner, "delivery attempt started");
    let started = std::time::Instant::now();
    let result = deliver(config, &delivery).await;
    match result {
        Ok(response) if response.status.is_success() => {
            let mut tx = pool.begin().await?;
            sqlx::query("UPDATE notification_deliveries SET status = 'delivered', delivered_at = now(), lease_owner = NULL, lease_until = NULL, attempt_count = attempt_count + 1, retry_attempt_count = retry_attempt_count + 1 WHERE id = $1").bind(delivery.id).execute(&mut *tx).await?;
            record_attempt(
                &mut tx,
                delivery.id,
                "delivered",
                i32::from(response.status.as_u16()),
                false,
                None,
                started.elapsed(),
            )
            .await?;
            if matches!(
                delivery.event_type.as_str(),
                "incident.detected" | "incident.started" | "incident.reopened"
            ) {
                crate::polling::events::fanout_resolved_incident_after_entry_delivery(
                    &mut tx,
                    delivery.entity_id,
                    delivery.lifecycle_generation,
                )
                .await?;
            }
            tx.commit().await?;
            tracing::info!(delivery_id = %delivery.id, notification_event_id = %delivery.event_id, http_status = response.status.as_u16(), "delivery confirmed");
        }
        Ok(response)
            if response.status == StatusCode::TOO_MANY_REQUESTS
                || response.status == StatusCode::REQUEST_TIMEOUT =>
        {
            retry(
                pool,
                delivery.id,
                format!("destination returned HTTP {}", response.status.as_u16()),
                i32::from(response.status.as_u16()),
                false,
                response.retry_after_seconds,
                started.elapsed(),
            )
            .await?
        }
        Ok(response) if response.status.is_client_error() => {
            let mut tx = pool.begin().await?;
            sqlx::query("UPDATE notification_deliveries SET status = 'failed', lease_owner = NULL, lease_until = NULL, attempt_count = attempt_count + 1, retry_attempt_count = retry_attempt_count + 1, last_error = $2 WHERE id = $1").bind(delivery.id).bind(format!("destination returned HTTP {}", response.status.as_u16())).execute(&mut *tx).await?;
            record_attempt(
                &mut tx,
                delivery.id,
                "failed",
                i32::from(response.status.as_u16()),
                false,
                Some(format!("HTTP {}", response.status.as_u16())),
                started.elapsed(),
            )
            .await?;
            tx.commit().await?;
            tracing::warn!(delivery_id = %delivery.id, notification_event_id = %delivery.event_id, http_status = response.status.as_u16(), "delivery failed permanently");
        }
        Ok(response) => {
            retry(
                pool,
                delivery.id,
                format!("destination returned HTTP {}", response.status.as_u16()),
                i32::from(response.status.as_u16()),
                false,
                response.retry_after_seconds,
                started.elapsed(),
            )
            .await?
        }
        Err(error) => {
            let ambiguous = error
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| error.is_timeout() && !error.is_connect());
            let message = sanitized_delivery_error(&error);
            retry(
                pool,
                delivery.id,
                message,
                0,
                ambiguous,
                None,
                started.elapsed(),
            )
            .await?
        }
    }
    Ok(true)
}

fn sanitized_delivery_error(error: &anyhow::Error) -> String {
    let Some(error) = error.downcast_ref::<reqwest::Error>() else {
        return "notification delivery failed before a response was received".into();
    };
    if error.is_timeout() {
        "notification request timed out".into()
    } else if error.is_connect() {
        "notification connection failed".into()
    } else if error.is_builder() || error.is_request() {
        "notification request could not be constructed or sent".into()
    } else {
        "notification transport failed".into()
    }
}

async fn claim(pool: &PgPool, owner: &str) -> Result<Option<Delivery>> {
    let mut tx = pool.begin().await?;
    if super::quiet_hours::prepare(&mut tx).await? {
        tx.commit().await?;
        return Ok(None);
    }
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Uuid,
            i32,
            DateTime<Utc>,
            Uuid,
            String,
            String,
            Value,
        ),
    >(
        "
        SELECT d.id, d.notification_event_id, e.event_type, e.entity_id,
               COALESCE(e.lifecycle_generation, 1), e.created_at,
               d.channel_id, c.channel_type, c.encrypted_config, e.payload
        FROM notification_deliveries d
        JOIN notification_channels c ON c.id = d.channel_id AND c.enabled AND c.deleted_at IS NULL
        LEFT JOIN alert_rules r ON r.id = d.alert_rule_id
        JOIN notification_events e ON e.id = d.notification_event_id
        WHERE d.status IN ('pending', 'retrying', 'ambiguous') AND d.next_attempt_at <= now()
          AND (d.lease_until IS NULL OR d.lease_until < now())
          AND (d.resend_requested_at IS NOT NULL OR d.alert_rule_id IS NULL OR (r.enabled AND r.deleted_at IS NULL))
        ORDER BY d.next_attempt_at, d.id
        FOR UPDATE OF d SKIP LOCKED LIMIT 1
    ",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some((
        id,
        event_id,
        event_type,
        entity_id,
        lifecycle_generation,
        event_created_at,
        channel_id,
        channel_type,
        encrypted_config,
        mut payload,
    )) = row
    else {
        tx.rollback().await?;
        return Ok(None);
    };
    enrich_payload(&mut tx, &event_type, entity_id, &mut payload).await?;
    sqlx::query("UPDATE notification_deliveries SET lease_owner = $2, lease_until = now() + $3 * interval '1 second' WHERE id = $1").bind(id).bind(owner).bind(DELIVERY_LEASE_SECONDS).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(Delivery {
        id,
        event_id,
        event_type,
        entity_id,
        lifecycle_generation,
        event_created_at,
        channel_id,
        channel_type,
        encrypted_config,
        payload,
    }))
}

async fn enrich_payload(
    tx: &mut Transaction<'_, Postgres>,
    event_type: &str,
    entity_id: Uuid,
    payload: &mut Value,
) -> Result<()> {
    let Value::Object(map) = payload else {
        return Ok(());
    };
    if event_type.starts_with("incident.") {
        let providers = sqlx::query_scalar::<_, Value>("SELECT COALESCE(jsonb_agg(jsonb_build_object('id', provider.id, 'name', provider.name, 'slug', provider.slug) ORDER BY provider.name), '[]'::jsonb) FROM incident_providers link JOIN providers provider ON provider.id = link.provider_id WHERE link.incident_id = $1")
            .bind(entity_id)
            .fetch_one(&mut **tx)
            .await?;
        let components = sqlx::query_scalar::<_, Value>("SELECT COALESCE(jsonb_agg(jsonb_build_object('id', component.id, 'name', component.name, 'status', current.normalized_status) ORDER BY component.position, component.name), '[]'::jsonb) FROM incident_components link JOIN components component ON component.id = link.component_id LEFT JOIN component_status_current current ON current.component_id = component.id WHERE link.incident_id = $1")
            .bind(entity_id)
            .fetch_one(&mut **tx)
            .await?;
        map.entry("providers").or_insert(providers);
        map.entry("components").or_insert(components);
    } else if event_type == "provider.status_changed" {
        if let Some(provider) = sqlx::query_scalar::<_, Value>("SELECT jsonb_build_object('id', id, 'name', name, 'slug', slug) FROM providers WHERE id = $1")
            .bind(entity_id)
            .fetch_optional(&mut **tx)
            .await?
        {
            map.entry("provider").or_insert(provider);
        }
    } else if event_type == "component.status_changed" {
        if let Some((component, provider)) = sqlx::query_as::<_, (Value, Value)>("SELECT jsonb_build_object('id', component.id, 'name', component.name), jsonb_build_object('id', provider.id, 'name', provider.name, 'slug', provider.slug) FROM components component JOIN providers provider ON provider.id = component.provider_id WHERE component.id = $1")
            .bind(entity_id)
            .fetch_optional(&mut **tx)
            .await?
        {
            map.entry("component").or_insert(component);
            map.entry("provider").or_insert(provider);
        }
    } else if event_type.starts_with("source.")
        && let Some(source) = sqlx::query_scalar::<_, Value>("SELECT jsonb_build_object('id', id, 'source_key', source_key, 'adapter', adapter) FROM provider_sources WHERE id = $1")
            .bind(entity_id)
            .fetch_optional(&mut **tx)
            .await?
    {
        map.entry("source").or_insert(source);
    }
    Ok(())
}

async fn deliver(config: &Config, delivery: &Delivery) -> Result<DeliveryResponse> {
    let key = config.encryption_key_bytes()?;
    let plaintext = secrets::open(
        &key,
        &format!("channel:{}:{}", delivery.channel_id, delivery.channel_type),
        &delivery.encrypted_config,
    )?;
    let channel_config = secrets::ChannelConfig::decode(&plaintext)?;
    let target = channel_config.target;
    let (mut url, pinned_address) = tokio::time::timeout(
        Duration::from_secs(5),
        safe_destination(
            &target,
            &delivery.channel_type,
            config.allow_private_notification_targets,
        ),
    )
    .await??;
    let mut client_builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(Duration::from_secs(15));
    if let (Some(host), Some(address)) = (url.host_str(), pinned_address) {
        client_builder = client_builder.resolve(host, address);
    }
    let client = client_builder.build()?;
    let mut body = render_delivery_payload(
        config,
        delivery.event_id,
        &delivery.event_type,
        delivery.entity_id,
        delivery.event_created_at,
        &delivery.channel_type,
        delivery.payload.clone(),
    );
    if delivery.channel_type == "ntfy" {
        let topic = super::ntfy_topic(&url)
            .ok_or_else(|| anyhow::anyhow!("ntfy destination is missing a valid topic"))?
            .to_owned();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("ntfy destination cannot be a base URL"))?
            .pop()
            .push("");
        let Value::Object(map) = &mut body else {
            bail!("ntfy payload must be an object");
        };
        map.insert("topic".into(), Value::String(topic));
    }
    let event_id = delivery.event_id.to_string();
    let zulip_content = body
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let body = serde_json::to_vec(&body)?;
    let timestamp = Utc::now().timestamp().to_string();
    let mut request = client
        .post(url)
        .header(
            "content-type",
            if delivery.channel_type == "zulip" {
                "application/x-www-form-urlencoded"
            } else {
                "application/json"
            },
        )
        .header("user-agent", "StatusDeck/0.1")
        .header("x-statusdeck-event-id", &event_id)
        .header("x-statusdeck-timestamp", &timestamp)
        .body(body.clone());
    if delivery.channel_type == "webhook" {
        if let Some(signing_secret) = channel_config.signing_secret.as_deref() {
            let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
                .map_err(|_| anyhow::anyhow!("could not create webhook signature"))?;
            mac.update(timestamp.as_bytes());
            mac.update(b".");
            mac.update(&body);
            request = request.header(
                "x-statusdeck-signature",
                format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
            );
        }
    } else if delivery.channel_type == "zulip" {
        let bot_email = channel_config
            .bot_email
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Zulip bot email is missing"))?;
        let token = channel_config
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Zulip API key is missing"))?;
        let stream = channel_config
            .stream
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Zulip channel is missing"))?;
        let topic = channel_config
            .topic
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Zulip topic is missing"))?;
        let content = zulip_content
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Zulip notification content is missing"))?;
        request = request.basic_auth(bot_email, Some(token)).form(&[
            ("type", "stream"),
            ("to", &serde_json::to_string(stream)?),
            ("topic", topic),
            ("content", content),
        ]);
    } else if delivery.channel_type == "gotify" {
        request = request.header(
            "x-gotify-key",
            channel_config
                .token
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Gotify application token is missing"))?,
        );
    } else if delivery.channel_type == "ntfy"
        && let Some(token) = channel_config.token.as_deref()
    {
        request = request.bearer_auth(token);
    } else if delivery.channel_type == "discord" {
        request = request.query(&[("wait", "true")]);
    }
    let response = request.send().await?;
    let retry_after_seconds = crate::retry_after::seconds(response.headers());
    Ok(DeliveryResponse {
        status: response.status(),
        retry_after_seconds,
    })
}

pub(crate) fn render_delivery_payload(
    config: &Config,
    event_id: Uuid,
    event_type: &str,
    entity_id: Uuid,
    event_created_at: DateTime<Utc>,
    channel_type: &str,
    mut payload: Value,
) -> Value {
    let event_id = event_id.to_string();
    if let Value::Object(map) = &mut payload {
        if !matches!(
            channel_type,
            "discord" | "slack" | "mattermost" | "gotify" | "ntfy" | "zulip"
        ) {
            map.remove("message");
        }
        map.remove("official_url");
        if let Some(Value::String(title)) = map.get_mut("title")
            && let Some(unprefixed) = title.strip_prefix("StatusDeck:")
        {
            *title = unprefixed.trim_start().to_owned();
        }
        map.insert("schema_version".into(), Value::String("1".into()));
        map.insert("event_id".into(), Value::String(event_id.clone()));
        map.insert("event_type".into(), Value::String(event_type.to_owned()));
        map.insert(
            "occurred_at".into(),
            Value::String(event_created_at.to_rfc3339()),
        );
        map.insert(
            "application_url".into(),
            Value::String(application_url(config, event_type, entity_id, &event_id)),
        );
        map.insert(
            "logo_url".into(),
            Value::String(
                config
                    .base_url
                    .join("statusdeck-logo.png")
                    .map_or_else(|_| config.base_url.to_string(), |url| url.to_string()),
            ),
        );
    }
    match channel_type {
        "discord" => super::discord::render(&payload),
        "slack" | "mattermost" => render_chat_attachment(&payload, &event_id, channel_type),
        "gotify" => json!({
            "title": notification_display_title(&payload),
            "message": render_gotify_content(&payload, usize::MAX),
            "extras": {
                "client::display": {"contentType": "text/plain"},
                "client::notification": {"click": {"url": payload.get("application_url")}}
            }
        }),
        "zulip" => json!({"content": render_zulip_content(&payload)}),
        "ntfy" => render_ntfy_payload(&payload),
        _ => {
            let content = render_notification_content(&payload, &event_id);
            if let Value::Object(map) = &mut payload {
                map.insert("content".into(), Value::String(content));
            }
            payload
        }
    }
}

fn render_zulip_content(payload: &Value) -> String {
    let title = notification_display_title(payload)
        .chars()
        .take(ZULIP_TITLE_LIMIT)
        .collect::<String>();
    let text = render_gotify_content(payload, usize::MAX);
    let url = payload.get("application_url").and_then(Value::as_str);
    let text = url.and_then(|url| text.strip_suffix(url)).unwrap_or(&text);
    let paragraphs = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(zulip_escape)
        .collect::<Vec<_>>()
        .join("\n\n");
    let suffix = url.map_or_else(String::new, |url| {
        format!(
            "\n\n[View in StatusDeck]({})",
            url.replace('(', "%28").replace(')', "%29")
        )
    });
    let heading = format!("**{}**\n\n", zulip_escape(&title));
    let available = ZULIP_MESSAGE_LIMIT - heading.encode_utf16().count();
    // Omit an oversized link rather than sending a broken, truncated URL.
    let suffix = if suffix.encode_utf16().count() < available {
        &suffix
    } else {
        ""
    };
    let mut content = heading;
    content.push_str(&super::truncate(
        &paragraphs,
        available - suffix.encode_utf16().count(),
    ));
    content.push_str(suffix);
    content
}

fn render_ntfy_payload(payload: &Value) -> Value {
    let mut body = json!({
        "title": truncate_bytes(&notification_display_title(payload), NTFY_TITLE_BYTES),
        "message": render_gotify_content(payload, NTFY_MESSAGE_BYTES),
        "priority": ntfy_priority(payload),
        "tags": ["statusdeck"],
        "click": payload.get("application_url")
    });
    // ntfy's JSON endpoint allows twice the message byte limit. Reserve the
    // topic's maximum 64 ASCII bytes plus its JSON key, quotes and comma.
    const TOPIC_JSON_BYTES: usize = 75;
    for field in ["message", "title", "click"] {
        let excess =
            (body.to_string().len() + TOPIC_JSON_BYTES).saturating_sub(NTFY_MESSAGE_BYTES * 2);
        if excess == 0 {
            break;
        }
        let Some(value) = body[field].as_str() else {
            continue;
        };
        let available = value.len().saturating_sub(excess);
        body[field] = match field {
            "message" => {
                let footer_bytes = value.rfind("\n\n").map_or(0, |index| value.len() - index);
                json!(render_gotify_content(payload, available.max(footer_bytes)))
            }
            "title" => json!(truncate_bytes(value, available)),
            _ => Value::Null,
        };
    }
    body
}

fn ntfy_priority(payload: &Value) -> u8 {
    match notification_state(payload) {
        "major_outage" => 5,
        "partial_outage" | "degraded" | "stale" | "failing" => 4,
        _ => 3,
    }
}

fn application_url(config: &Config, event_type: &str, entity_id: Uuid, event_id: &str) -> String {
    let path = if event_type == "system.quiet_summary" {
        format!("notifications/summaries/{event_id}")
    } else if event_type.starts_with("incident.") {
        format!("incidents/{entity_id}")
    } else if event_type == "provider.status_changed" {
        format!("catalog/{entity_id}")
    } else {
        "system".to_owned()
    };
    config
        .base_url
        .join(&path)
        .map_or_else(|_| config.base_url.to_string(), |url| url.to_string())
}

async fn safe_destination(
    target: &str,
    channel_type: &str,
    allow_private: bool,
) -> Result<(reqwest::Url, Option<SocketAddr>)> {
    let url = reqwest::Url::parse(target)?;
    if url.scheme() != "https" || url.host_str().is_none() {
        bail!("notification destinations must use HTTPS and include a host");
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        bail!("notification destinations cannot contain credentials or fragments");
    }
    if !is_valid_channel_target(channel_type, &url) {
        bail!("notification destination does not match its channel type");
    }
    if let Some(host) = url.host_str() {
        if !allow_private
            && (host.eq_ignore_ascii_case("localhost")
                || host.eq_ignore_ascii_case("metadata.google.internal")
                || host.ends_with(".localhost")
                || host.ends_with(".internal"))
        {
            bail!("private notification destination is blocked");
        }
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses = tokio::net::lookup_host((host.trim_matches(['[', ']']), port)).await?;
        let mut public_address = None;
        for address in addresses {
            let ip = address.ip();
            if !allow_private && is_non_public_ip(ip) {
                bail!("private notification destination is blocked");
            }
            public_address.get_or_insert(address);
        }
        if public_address.is_none() {
            bail!("notification destination did not resolve to an address");
        }
        return Ok((url, public_address));
    }
    Ok((url, None))
}

pub(crate) fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(value) => {
            let [first, second, _, _] = value.octets();
            value.is_private()
                || value.is_loopback()
                || value.is_link_local()
                || value.is_unspecified()
                || value.is_broadcast()
                || value.is_multicast()
                || value.is_documentation()
                || first == 0
                || (first == 100 && (64..=127).contains(&second))
                || (first == 198 && (18..=19).contains(&second))
        }
        IpAddr::V6(value) => {
            let first = value.segments()[0];
            value.is_loopback()
                || value.is_unspecified()
                || value.is_unique_local()
                || value.is_multicast()
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0
                || (value.segments()[0] == 0x2001 && value.segments()[1] == 0x0db8)
                || value
                    .to_ipv4()
                    .is_some_and(|mapped| is_non_public_ip(IpAddr::V4(mapped)))
        }
    }
}

pub(super) fn notification_title(payload: &Value) -> &str {
    payload
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("provider")
                .and_then(|provider| provider.get("name"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            payload
                .get("component")
                .and_then(|component| component.get("name"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            payload
                .get("source")
                .and_then(|source| source.get("source_key"))
                .and_then(Value::as_str)
        })
        .unwrap_or("StatusDeck event")
}

pub(super) fn notification_display_title(payload: &Value) -> String {
    let title = notification_title(payload).trim();
    if payload.get("event_type").and_then(Value::as_str) == Some("system.quiet_summary") {
        // Normalize saved summary titles as well as newly generated ones.
        return title.split_once(':').map_or_else(
            || "Quiet-Hours Summary".to_owned(),
            |(_, name)| format!("Quiet-Hours Summary: {}", capitalize(name.trim())),
        );
    }
    capitalize(title)
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub(super) fn notification_description(message: &str) -> Option<String> {
    let rendered = if message.contains('<') {
        match html2text::config::plain()
            .allow_width_overflow()
            .string_from_read(message.as_bytes(), 1500)
        {
            Ok(rendered) => rendered,
            Err(error) => {
                tracing::warn!(%error, "could not render notification update HTML");
                return None;
            }
        }
    } else {
        message.to_owned()
    };
    (!rendered.trim().is_empty()).then_some(rendered)
}

pub(super) fn notification_state(payload: &Value) -> &str {
    payload
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| payload.get("lifecycle").and_then(Value::as_str))
        .or_else(|| payload.get("freshness").and_then(Value::as_str))
        .unwrap_or_else(|| {
            payload
                .get("event_type")
                .and_then(Value::as_str)
                .unwrap_or("changed")
        })
}

pub(super) fn notification_color(payload: &Value) -> u32 {
    let state = notification_state(payload);
    let severity = payload.get("severity").and_then(Value::as_str);
    match state {
        "resolved" | "operational" | "fresh" | "source.recovered" => 0x22_c5_5e,
        "major_outage" | "failing" => 0xef_44_44,
        _ if matches!(severity, Some("major" | "critical")) => 0xef_44_44,
        "open" | "resolution_pending" | "degraded" | "partial_outage" | "stale" | "delayed"
        | "source.stale" => 0xf5_9e_0b,
        _ => 0x3b_82_f6,
    }
}

pub(super) fn notification_label(value: &str) -> String {
    let value = value.trim().replace(['_', '.'], " ");
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown".to_owned(),
    }
}

pub(super) fn notification_names(payload: &Value, plural: &str, singular: &str) -> String {
    let names = payload
        .get(plural)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if !names.is_empty() {
        return names.join("\n");
    }
    payload
        .get(singular)
        .and_then(|item| item.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn render_chat_attachment(payload: &Value, event_id: &str, channel_type: &str) -> Value {
    let mut fields = vec![json!({
        "title": "Status",
        "value": super::truncate(&notification_label(notification_state(payload)), CHAT_FIELD_LIMIT),
        "short": true
    })];
    if let Some(severity) = payload
        .get("severity")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        fields.push(json!({
            "title": "Severity",
            "value": super::truncate(&notification_label(severity), CHAT_FIELD_LIMIT),
            "short": true
        }));
    }
    let providers = notification_names(payload, "providers", "provider");
    if !providers.is_empty() {
        fields.push(json!({"title": "Providers", "value": super::truncate(&providers, CHAT_FIELD_LIMIT), "short": true}));
    }
    let components = notification_names(payload, "components", "component");
    if !components.is_empty() {
        fields.push(json!({"title": "Components", "value": super::truncate(&components, CHAT_FIELD_LIMIT), "short": false}));
    }
    let timestamp = payload
        .get("occurred_at")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp());
    let footer = if channel_type == "mattermost" {
        payload
            .get("occurred_at")
            .and_then(Value::as_str)
            .map_or_else(
                || "StatusDeck".to_owned(),
                |value| format!("StatusDeck · {value}"),
            )
    } else {
        "StatusDeck".to_owned()
    };
    let text = (payload.get("event_type").and_then(Value::as_str) != Some("system.quiet_summary"))
        .then(|| {
            payload
                .get("message")
                .and_then(Value::as_str)
                .and_then(notification_description)
        })
        .flatten()
        .map(|text| super::truncate(&text, CHAT_UPDATE_LIMIT));
    let mut attachment = json!({
        "fallback": render_notification_content(payload, event_id),
        "color": format!("#{:06x}", notification_color(payload)),
        "title": super::truncate(&notification_display_title(payload), CHAT_TITLE_LIMIT),
        "title_link": payload.get("application_url"),
        "text": text,
        "fields": fields,
        "footer": footer,
        "ts": (channel_type == "slack").then_some(timestamp).flatten()
    });
    if let Value::Object(map) = &mut attachment {
        map.retain(|_, value| !value.is_null());
    }
    json!({
        "attachments": [attachment],
        "icon_url": payload.get("logo_url")
    })
}

fn render_gotify_content(payload: &Value, max_bytes: usize) -> String {
    let mut lines = vec![format!(
        "Status: {}",
        notification_label(notification_state(payload))
    )];
    if let Some(severity) = payload
        .get("severity")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("Severity: {}", notification_label(severity)));
    }
    for (label, plural, singular) in [
        ("Providers", "providers", "provider"),
        ("Components", "components", "component"),
    ] {
        let names = notification_names(payload, plural, singular);
        if !names.is_empty() {
            lines.push(format!("{label}: {}", names.replace('\n', ", ")));
        }
    }
    if payload.get("event_type").and_then(Value::as_str) != Some("system.quiet_summary")
        && let Some(description) = payload
            .get("message")
            .and_then(Value::as_str)
            .and_then(notification_description)
    {
        lines.push(String::new());
        lines.push(description);
    }
    let footer = payload
        .get("occurred_at")
        .and_then(Value::as_str)
        .map_or_else(
            || "StatusDeck".to_owned(),
            |value| format!("StatusDeck · {value}"),
        );
    let mut suffix = payload
        .get("application_url")
        .and_then(Value::as_str)
        .map_or_else(
            || format!("\n\n{footer}"),
            |url| format!("\n\n{footer}\n{url}"),
        );
    if suffix.chars().count() >= PUSH_MESSAGE_CHARACTERS || suffix.len() > max_bytes {
        suffix = format!("\n\n{footer}");
    }
    let available = PUSH_MESSAGE_CHARACTERS.saturating_sub(suffix.chars().count());
    let content = lines.join("\n").chars().take(available).collect::<String>();
    let suffix = truncate_bytes(&suffix, max_bytes);
    let mut content = truncate_bytes(&content, max_bytes - suffix.len()).to_owned();
    content.push_str(suffix);
    content
}

fn truncate_bytes(value: &str, limit: usize) -> &str {
    let mut end = limit.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn render_notification_content(payload: &Value, event_id: &str) -> String {
    let title = notification_title(payload);
    let state = notification_state(payload);
    let mut content = format!("StatusDeck: {title} — {state} (event {event_id})");
    let providers = payload
        .get("providers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|provider| provider.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if !providers.is_empty() {
        content.push_str(&format!("\nProviders: {}", providers.join(", ")));
    } else if let Some(provider) = payload
        .get("provider")
        .and_then(|provider| provider.get("name"))
        .and_then(Value::as_str)
    {
        content.push_str(&format!("\nProvider: {provider}"));
    }
    let components = payload
        .get("components")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|component| {
            let name = component.get("name")?.as_str()?;
            let status = component.get("status").and_then(Value::as_str);
            Some(status.map_or_else(|| name.to_owned(), |status| format!("{name} ({status})")))
        })
        .collect::<Vec<_>>();
    if !components.is_empty() {
        content.push_str(&format!("\nComponents: {}", components.join(", ")));
    } else if let Some(component) = payload
        .get("component")
        .and_then(|component| component.get("name"))
        .and_then(Value::as_str)
    {
        content.push_str(&format!("\nComponent: {component}"));
    }
    let suffix = payload
        .get("application_url")
        .and_then(Value::as_str)
        .map_or_else(String::new, |url| format!("\n{url}"));
    let available = 1900_usize.saturating_sub(suffix.chars().count());
    let mut rendered = content.chars().take(available).collect::<String>();
    rendered.push_str(&suffix);
    rendered.chars().take(1900).collect()
}

async fn retry(
    pool: &PgPool,
    id: Uuid,
    error: String,
    status: i32,
    ambiguous: bool,
    retry_after_seconds: Option<u64>,
    duration: Duration,
) -> Result<()> {
    let retry_delay = retry_after_seconds.map_or(0, |seconds| seconds.min(3600));
    let mut tx = pool.begin().await?;
    let delivery_status = sqlx::query_scalar::<_, String>("UPDATE notification_deliveries SET status = CASE WHEN retry_attempt_count >= 4 THEN 'failed' WHEN $4 THEN 'ambiguous' ELSE 'retrying' END, next_attempt_at = now() + (CASE WHEN $3 > 0 THEN $3 ELSE GREATEST(1, floor(random() * CASE retry_attempt_count WHEN 0 THEN 60 WHEN 1 THEN 300 WHEN 2 THEN 900 ELSE 3600 END)::bigint) END) * interval '1 second', lease_owner = NULL, lease_until = NULL, attempt_count = attempt_count + 1, retry_attempt_count = retry_attempt_count + 1, last_error = $2 WHERE id = $1 AND status IN ('pending', 'retrying', 'ambiguous') RETURNING status")
        .bind(id)
        .bind(&error)
        .bind(i64::try_from(retry_delay).unwrap_or(3_600))
        .bind(ambiguous)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(delivery_status) = delivery_status else {
        tx.rollback().await?;
        return Ok(());
    };
    record_attempt(
        &mut tx,
        id,
        &delivery_status,
        status,
        ambiguous,
        Some(error),
        duration,
    )
    .await?;
    tx.commit().await?;
    let response_status = (status != 0).then_some(status);
    tracing::warn!(delivery_id = %id, status = %delivery_status, ?response_status, ambiguous, "delivery scheduled after failure");
    Ok(())
}

async fn record_attempt(
    tx: &mut Transaction<'_, Postgres>,
    delivery_id: Uuid,
    outcome: &str,
    status: i32,
    ambiguous: bool,
    error: Option<String>,
    duration: Duration,
) -> Result<()> {
    let response_class = if status == 0 {
        "transport"
    } else {
        match status / 100 {
            2 => "success",
            4 => "client_error",
            5 => "server_error",
            _ => "other",
        }
    };
    sqlx::query("INSERT INTO notification_attempts (delivery_id, duration_ms, outcome, response_class, response_status, error_message, ambiguous) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(delivery_id)
        .bind(i32::try_from(duration.as_millis()).unwrap_or(i32::MAX))
        .bind(outcome)
        .bind(response_class)
        .bind((status != 0).then_some(status))
        .bind(error)
        .bind(ambiguous)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn zulip_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '*' | '_' | '`' | '[' | ']' | '~' | '>' | '#' | '!'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
