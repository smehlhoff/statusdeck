use std::{env, fmt, fs, net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use url::Url;

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub base_url: Url,
    pub bind: SocketAddr,
    pub log_level: String,
    pub log_format: String,
    pub secret_key: Secret,
    pub encryption_key: Secret,
    pub admin_email: String,
    pub admin_password: Option<Secret>,
    pub poll_interval: Duration,
    pub stale_multiplier: u32,
    pub global_poll_concurrency: usize,
    pub per_host_concurrency: usize,
    pub raw_payload_retention_days: u32,
    pub trusted_hosts: Vec<String>,
    pub session_cookie_secure: bool,
    pub trust_proxy: bool,
    pub allow_private_notification_targets: bool,
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("database_url", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("bind", &self.bind)
            .field("log_level", &self.log_level)
            .field("log_format", &self.log_format)
            .field("secret_key", &self.secret_key)
            .field("encryption_key", &self.encryption_key)
            .field("admin_email", &self.admin_email)
            .field("admin_password", &self.admin_password)
            .field("poll_interval", &self.poll_interval)
            .field("stale_multiplier", &self.stale_multiplier)
            .field("global_poll_concurrency", &self.global_poll_concurrency)
            .field("per_host_concurrency", &self.per_host_concurrency)
            .field(
                "raw_payload_retention_days",
                &self.raw_payload_retention_days,
            )
            .field("trusted_hosts", &self.trusted_hosts)
            .field("session_cookie_secure", &self.session_cookie_secure)
            .field("trust_proxy", &self.trust_proxy)
            .field(
                "allow_private_notification_targets",
                &self.allow_private_notification_targets,
            )
            .finish()
    }
}

impl Config {
    pub fn database_url_from_env() -> Result<String> {
        secret_or_direct("STATUSDECK_DATABASE_URL", "STATUSDECK_DATABASE_URL_FILE")
    }

    pub fn encryption_key_bytes(&self) -> Result<Vec<u8>> {
        decode_key(self.encryption_key.expose())
    }

    pub fn from_env() -> Result<Self> {
        let database_url = Self::database_url_from_env()?;
        let base_value =
            optional("STATUSDECK_BASE_URL").unwrap_or_else(|| "http://localhost".to_owned());
        let base_url = parse_url("STATUSDECK_BASE_URL", &base_value)?;
        let bind = optional("STATUSDECK_BIND")
            .unwrap_or_else(|| "127.0.0.1:8080".to_owned())
            .parse()
            .context("STATUSDECK_BIND must be a host:port address")?;
        let log_level = optional("STATUSDECK_LOG_LEVEL").unwrap_or_else(|| "info".to_owned());
        let log_format = optional("STATUSDECK_LOG_FORMAT").unwrap_or_else(|| "pretty".to_owned());
        if !matches!(log_format.as_str(), "pretty" | "json") {
            bail!("STATUSDECK_LOG_FORMAT must be pretty or json");
        }
        let secret_key = secret("STATUSDECK_SECRET_KEY", "STATUSDECK_SECRET_KEY_FILE")?;
        let encryption_key = secret(
            "STATUSDECK_ENCRYPTION_KEY",
            "STATUSDECK_ENCRYPTION_KEY_FILE",
        )?;
        if secret_key.expose().len() < 32 {
            bail!("STATUSDECK_SECRET_KEY must be at least 32 bytes");
        }
        let encryption_raw = decode_key(encryption_key.expose())
            .context("STATUSDECK_ENCRYPTION_KEY must be base64 or 32 raw bytes")?;
        if encryption_raw.len() != 32 {
            bail!("STATUSDECK_ENCRYPTION_KEY must decode to 32 bytes");
        }
        let admin_email = normalize_email(&secret_or_direct(
            "STATUSDECK_ADMIN_EMAIL",
            "STATUSDECK_ADMIN_EMAIL_FILE",
        )?)?;
        let admin_password = optional_secret(
            "STATUSDECK_ADMIN_PASSWORD",
            "STATUSDECK_ADMIN_PASSWORD_FILE",
        )?;
        if admin_password
            .as_ref()
            .is_some_and(|password| !(12..=1024).contains(&password.expose().chars().count()))
        {
            bail!("STATUSDECK_ADMIN_PASSWORD must contain between 12 and 1,024 characters");
        }
        let poll_interval = duration_seconds("STATUSDECK_DEFAULT_POLL_INTERVAL", 300)?;
        if poll_interval > Duration::from_secs(86_400) {
            bail!("STATUSDECK_DEFAULT_POLL_INTERVAL cannot exceed 86,400 seconds");
        }
        let stale_multiplier = positive_u32("STATUSDECK_STALE_MULTIPLIER", 3)?;
        if stale_multiplier > 100 {
            bail!("STATUSDECK_STALE_MULTIPLIER cannot exceed 100");
        }
        let global_poll_concurrency = positive_usize("STATUSDECK_GLOBAL_POLL_CONCURRENCY", 10)?;
        let per_host_concurrency = positive_usize("STATUSDECK_PER_HOST_CONCURRENCY", 2)?;
        if global_poll_concurrency > 100 || per_host_concurrency > 100 {
            bail!("poll concurrency cannot exceed 100");
        }
        if per_host_concurrency > global_poll_concurrency {
            bail!(
                "STATUSDECK_PER_HOST_CONCURRENCY cannot exceed STATUSDECK_GLOBAL_POLL_CONCURRENCY"
            );
        }
        let raw_payload_retention_days = positive_u32("STATUSDECK_RAW_PAYLOAD_RETENTION_DAYS", 7)?;
        if raw_payload_retention_days > 3_650 {
            bail!("STATUSDECK_RAW_PAYLOAD_RETENTION_DAYS cannot exceed 3,650");
        }
        let trusted_hosts = optional("STATUSDECK_TRUSTED_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        let session_cookie_secure = boolean("STATUSDECK_SESSION_COOKIE_SECURE", false)?;
        if base_url.scheme() == "https" && !session_cookie_secure {
            bail!(
                "STATUSDECK_SESSION_COOKIE_SECURE must be true when STATUSDECK_BASE_URL uses HTTPS"
            );
        }
        let trust_proxy = boolean("STATUSDECK_TRUST_PROXY", false)?;
        let allow_private_notification_targets =
            boolean("STATUSDECK_ALLOW_PRIVATE_NOTIFICATION_TARGETS", false)?;
        Ok(Self {
            database_url,
            base_url,
            bind,
            log_level,
            log_format,
            secret_key,
            encryption_key,
            admin_email,
            admin_password,
            poll_interval,
            stale_multiplier,
            global_poll_concurrency,
            per_host_concurrency,
            raw_payload_retention_days,
            trusted_hosts,
            session_cookie_secure,
            trust_proxy,
            allow_private_notification_targets,
        })
    }
}

pub fn normalize_email(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if !(3..=254).contains(&normalized.len())
        || !normalized.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        })
        || normalized
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        bail!("administrator email must be a valid email address");
    }
    Ok(normalized)
}

pub(crate) fn optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

pub(crate) fn secret(value_name: &str, file_name: &str) -> Result<Secret> {
    optional_secret(value_name, file_name)?
        .with_context(|| format!("{value_name} or {file_name} is required"))
}

fn optional_secret(value_name: &str, file_name: &str) -> Result<Option<Secret>> {
    if let Some(path) = optional(file_name) {
        let contents = fs::read_to_string(PathBuf::from(path))
            .with_context(|| format!("could not read {file_name}"))?;
        return Ok(Some(Secret(
            contents.trim_end_matches(['\r', '\n']).to_owned(),
        )));
    }
    Ok(optional(value_name).map(Secret))
}

fn secret_or_direct(value_name: &str, file_name: &str) -> Result<String> {
    if let Some(path) = optional(file_name) {
        let contents =
            fs::read_to_string(path).with_context(|| format!("could not read {file_name}"))?;
        return Ok(contents.trim().to_owned());
    }
    optional(value_name).with_context(|| format!("{value_name} or {file_name} is required"))
}

fn parse_url(name: &str, value: &str) -> Result<Url> {
    let url = Url::parse(value).with_context(|| format!("{name} must be a URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("{name} must use http or https");
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        bail!("{name} must have a host and cannot contain credentials");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        bail!("{name} must be an origin URL without a path, query, or fragment");
    }
    Ok(url)
}

fn duration_seconds(name: &str, default: u64) -> Result<Duration> {
    let seconds = optional(name).map_or(Ok(default), |value| {
        value
            .parse()
            .with_context(|| format!("{name} must be seconds"))
    })?;
    if seconds == 0 {
        bail!("{name} must be positive");
    }
    Ok(Duration::from_secs(seconds))
}
fn positive_u32(name: &str, default: u32) -> Result<u32> {
    let value = optional(name).map_or(Ok(default), |v| {
        v.parse()
            .with_context(|| format!("{name} must be a positive integer"))
    })?;
    if value == 0 {
        bail!("{name} must be positive");
    }
    Ok(value)
}
fn positive_usize(name: &str, default: usize) -> Result<usize> {
    let value = optional(name).map_or(Ok(default), |v| {
        v.parse()
            .with_context(|| format!("{name} must be a positive integer"))
    })?;
    if value == 0 {
        bail!("{name} must be positive");
    }
    Ok(value)
}
fn boolean(name: &str, default: bool) -> Result<bool> {
    optional(name).map_or(Ok(default), |v| {
        bool::from_str(&v).with_context(|| format!("{name} must be true or false"))
    })
}
fn decode_key(value: &str) -> Result<Vec<u8>> {
    match BASE64.decode(value) {
        Ok(decoded) if decoded.len() == 32 => Ok(decoded),
        _ if value.len() == 32 => Ok(value.as_bytes().to_vec()),
        Ok(decoded) => Ok(decoded),
        Err(error) => Err(error.into()),
    }
}
