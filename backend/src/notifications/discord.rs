use super::truncate;
use serde_json::{Value, json};

use super::dispatcher::{
    notification_color, notification_description, notification_display_title, notification_label,
    notification_names, notification_state,
};

const TITLE_LIMIT: usize = 256;
const FIELD_LIMIT: usize = 1024;
const UPDATE_LIMIT: usize = 1500;
pub(super) fn render(payload: &Value) -> Value {
    let quiet_summary =
        payload.get("event_type").and_then(Value::as_str) == Some("system.quiet_summary");
    let state = notification_state(payload);
    let severity = payload.get("severity").and_then(Value::as_str);
    let color = notification_color(payload);

    // Four bounded fields, update, title and footer stay below Discord's 6,000-character total.
    let mut fields = Vec::with_capacity(4);
    fields.push(
        json!({"name": "Status", "value": truncate(&notification_label(state), FIELD_LIMIT), "inline": true}),
    );
    if let Some(severity) = severity.filter(|value| !value.trim().is_empty()) {
        fields.push(json!({"name": "Severity", "value": truncate(&notification_label(severity), FIELD_LIMIT), "inline": true}));
    }

    let providers = notification_names(payload, "providers", "provider");
    if !providers.is_empty() {
        fields.push(json!({"name": "Providers", "value": truncate(&providers, FIELD_LIMIT), "inline": true}));
    }
    // Component statuses are current snapshots, which may differ from the incident's state.
    // Show the affected names here; the linked incident page has the detailed statuses.
    let components = notification_names(payload, "components", "component");
    if !components.is_empty() {
        fields.push(json!({"name": "Components", "value": truncate(&components, FIELD_LIMIT), "inline": false}));
    }

    let mut embed = json!({
        "title": truncate(&notification_display_title(payload), TITLE_LIMIT),
        "color": color,
        "fields": fields,
        "footer": {"text": "StatusDeck"},
    });
    if !quiet_summary
        && let Some(update) = payload
            .get("message")
            .and_then(Value::as_str)
            .and_then(notification_description)
    {
        embed["description"] = json!(truncate(&update, UPDATE_LIMIT));
    }
    if let Some(url) = payload.get("application_url").and_then(Value::as_str) {
        embed["url"] = json!(url);
    }
    if let Some(timestamp) = payload.get("occurred_at").and_then(Value::as_str) {
        embed["timestamp"] = json!(timestamp);
    }
    json!({
        "embeds": [embed],
        "allowed_mentions": {"parse": []},
        "avatar_url": payload.get("logo_url")
    })
}
