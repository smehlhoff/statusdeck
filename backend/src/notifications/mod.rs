mod discord;
pub mod dispatcher;
pub(crate) mod quiet_hours;
pub mod secrets;

fn truncate(value: &str, limit: usize) -> String {
    let value = value.trim();
    // UTF-16 counting also keeps emoji within character-based service limits.
    if value.encode_utf16().count() <= limit {
        return value.to_owned();
    }
    let mut remaining = limit.saturating_sub(3);
    let mut result = String::new();
    for character in value.chars() {
        let width = character.len_utf16();
        if width > remaining {
            break;
        }
        result.push(character);
        remaining -= width;
    }
    result.extend("...".chars().take(limit));
    result
}

pub(crate) fn is_valid_channel_target(channel_type: &str, url: &url::Url) -> bool {
    match channel_type {
        "webhook" => true,
        "discord" => {
            url.host_str() == Some("discord.com")
                && is_valid_discord_webhook_path(url.path())
                && url.query().is_none()
        }
        "slack" => {
            let mut segments = url.path().trim_matches('/').split('/');
            matches!(
                url.host_str(),
                Some("hooks.slack.com" | "hooks.slack-gov.com")
            ) && matches!(
                (
                    segments.next(),
                    segments.next(),
                    segments.next(),
                    segments.next(),
                    segments.next()
                ),
                (Some("services"), Some(workspace), Some(channel), Some(token), None)
                    if !workspace.is_empty() && !channel.is_empty() && !token.is_empty()
            ) && url.query().is_none()
        }
        "mattermost" => {
            let mut segments = url.path().trim_matches('/').rsplit('/');
            matches!((segments.next(), segments.next()), (Some(token), Some("hooks")) if !token.is_empty())
                && url.query().is_none()
        }
        "gotify" => {
            url.path().trim_matches('/').rsplit('/').next() == Some("message")
                && url.query().is_none()
        }
        "zulip" => url.path() == "/api/v1/messages" && url.query().is_none(),
        "ntfy" => ntfy_topic(url).is_some(),
        _ => false,
    }
}

pub(crate) fn ntfy_topic(url: &url::Url) -> Option<&str> {
    if url.path().ends_with('/') || url.query().is_some() {
        return None;
    }
    url.path_segments()?.next_back().filter(|topic| {
        (1..=64).contains(&topic.len())
            && topic
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

pub(crate) fn is_valid_discord_webhook_path(path: &str) -> bool {
    let mut segments = path.trim_matches('/').split('/');
    matches!(
        (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next()
        ),
        (Some("api"), Some("webhooks"), Some(id), Some(token), None)
            if !id.is_empty() && !token.is_empty()
    )
}
