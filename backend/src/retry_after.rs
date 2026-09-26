use chrono::{DateTime, Utc};

pub(crate) fn seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    value.parse().ok().or_else(|| {
        let retry_at = DateTime::parse_from_rfc2822(value)
            .ok()?
            .with_timezone(&Utc);
        u64::try_from(retry_at.signed_duration_since(Utc::now()).num_seconds()).ok()
    })
}
