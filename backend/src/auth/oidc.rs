use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse},
};
use openidconnect::{
    AuthType, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, HttpRequest, HttpResponse, IdToken, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{
        CoreAuthenticationFlow, CoreClient, CoreGenderClaim, CoreJsonWebKeySet,
        CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm, CoreProviderMetadata,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::Mutex;
use url::Url;

use crate::{config::Config, notifications::secrets};
use zeroize::Zeroizing;

const RESPONSE_LIMIT: usize = 1024 * 1024;
const RETRY_INTERVAL: Duration = Duration::from_secs(60);
const METADATA_MAX_AGE: Duration = Duration::from_secs(60);
type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

const CONFIGURATION_CONTEXT: &str = "oidc-provider-configuration";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Configuration {
    pub enabled: bool,
    pub label: String,
    pub issuer_url: String,
    pub discovery_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub token_auth_method: String,
    pub allowed_endpoint_origins: Vec<String>,
    pub session_max_age_seconds: i64,
    pub ca_certificate_pem: String,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            enabled: false,
            label: "Single sign-on".to_owned(),
            issuer_url: String::new(),
            discovery_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            token_auth_method: "client_secret_basic".to_owned(),
            allowed_endpoint_origins: Vec::new(),
            session_max_age_seconds: 28_800,
            ca_certificate_pem: String::new(),
        }
    }
}

impl Configuration {
    pub fn decode(config: &Config, value: Option<&str>) -> Result<Self> {
        match value {
            Some(value) => {
                let plaintext = Zeroizing::new(secrets::open(
                    &config.encryption_key_bytes()?,
                    CONFIGURATION_CONTEXT,
                    value,
                )?);
                serde_json::from_str(&plaintext).context("Invalid saved OIDC configuration")
            }
            None => Ok(Self::default()),
        }
    }

    pub fn encode(&self, config: &Config) -> Result<String> {
        let plaintext = Zeroizing::new(serde_json::to_string(self)?);
        secrets::seal(
            &config.encryption_key_bytes()?,
            CONFIGURATION_CONTEXT,
            &plaintext,
        )
    }

    pub fn validate(&self, config: &Config) -> Result<()> {
        ensure!(
            !self.label.trim().is_empty()
                && self.label.chars().count() <= 100
                && !self.label.chars().any(char::is_control),
            "Provider label must contain 1–100 characters."
        );
        ensure!(
            self.issuer_url.len() <= 2048 && self.discovery_url.len() <= 2048,
            "Provider URLs must be at most 2048 characters."
        );
        ensure!(
            self.client_id.len() <= 1024 && self.client_secret.len() <= 8192,
            "Client ID or secret is too long."
        );
        ensure!(
            self.allowed_endpoint_origins.len() <= 16
                && self
                    .allowed_endpoint_origins
                    .iter()
                    .all(|s| s.len() <= 2048),
            "Use at most 16 additional endpoint origins, each at most 2048 characters."
        );
        ensure!(
            self.ca_certificate_pem.len() <= 65_536,
            "CA certificate is too large."
        );
        ensure!(
            (1..=28_800).contains(&self.session_max_age_seconds),
            "Session lifetime must be between 1 and 28800 seconds."
        );
        ensure!(
            matches!(
                self.token_auth_method.as_str(),
                "client_secret_basic" | "client_secret_post"
            ),
            "Unsupported client authentication method."
        );
        if self.enabled {
            Settings::from_configuration(config, self)?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RuntimeCache {
    current: Mutex<Option<(i64, Arc<Runtime>)>>,
}

impl RuntimeCache {
    pub async fn get(&self, config: &Config, pool: &PgPool) -> Result<Arc<Runtime>> {
        let mut current = self.current.lock().await;
        let (revision, encrypted) = sqlx::query_as::<_, (i64, Option<String>)>(
            "SELECT revision, configuration FROM oidc_configuration WHERE singleton",
        )
        .fetch_one(pool)
        .await?;
        if let Some((cached_revision, runtime)) = &*current
            && *cached_revision == revision
        {
            return Ok(runtime.clone());
        }
        let configuration = Configuration::decode(config, encrypted.as_deref())?;
        let mut runtime = Runtime::new(config, configuration);
        runtime.revision = revision;
        let runtime = Arc::new(runtime);
        *current = Some((revision, runtime.clone()));
        Ok(runtime)
    }
}

pub(crate) struct Settings {
    pub issuer: String,
    client_id: String,
    client_secret: Zeroizing<String>,
    discovery: Option<Url>,
    callback: Url,
    auth_type: AuthType,
    allowed_origins: Vec<String>,
    ca_certificate: Option<reqwest::Certificate>,
    pub max_age: i64,
    pub identity_key: String,
    pub flow_key: String,
}

impl Settings {
    fn from_configuration(config: &Config, input: &Configuration) -> Result<Self> {
        let issuer = input.issuer_url.trim().to_owned();
        let issuer_url = Url::parse(&issuer).context("Enter a valid issuer URL.")?;
        let client_id = input.client_id.trim().to_owned();
        ensure!(!client_id.is_empty(), "Client ID is required.");
        let client_secret = Zeroizing::new(input.client_secret.clone());
        ensure!(!client_secret.is_empty(), "Client secret is required.");
        let auth_type = match input.token_auth_method.as_str() {
            "client_secret_basic" => AuthType::BasicAuth,
            "client_secret_post" => AuthType::RequestBody,
            _ => bail!("Unsupported client authentication method."),
        };
        let max_age = input.session_max_age_seconds;
        validate_transport(&config.base_url).context("SSO requires an HTTPS application URL.")?;
        validate_transport(&issuer_url)?;
        ensure!(issuer_url.query().is_none(), "issuer query forbidden");
        let mut allowed_origins = vec![issuer_url.origin().ascii_serialization()];
        for origin in &input.allowed_endpoint_origins {
            let url = Url::parse(origin)?;
            validate_transport(&url)?;
            ensure!(
                url.path() == "/" && url.query().is_none(),
                "expected endpoint origin"
            );
            allowed_origins.push(url.origin().ascii_serialization());
        }
        let discovery = if input.discovery_url.trim().is_empty() {
            None
        } else {
            Some(Url::parse(input.discovery_url.trim()).context("Enter a valid discovery URL.")?)
        };
        let callback = config.base_url.join("/api/v1/auth/oidc/callback")?;
        let ca_pem = input.ca_certificate_pem.as_bytes();
        let ca_certificate = if ca_pem.is_empty() {
            None
        } else {
            Some(
                reqwest::Certificate::from_pem(ca_pem)
                    .context("Enter a valid PEM CA certificate.")?,
            )
        };
        let identity_key = fingerprint(&[&issuer, &client_id]);
        let flow_key = fingerprint(&[
            &identity_key,
            client_secret.as_str(),
            callback.as_str(),
            discovery.as_ref().map_or("", Url::as_str),
            if matches!(auth_type, AuthType::BasicAuth) {
                "basic"
            } else {
                "post"
            },
            &allowed_origins.join(","),
            &max_age.to_string(),
            &hex::encode(Sha256::digest(ca_pem)),
        ]);
        let settings = Self {
            issuer,
            client_id,
            client_secret,
            discovery,
            callback,
            auth_type,
            allowed_origins,
            ca_certificate,
            max_age,
            identity_key,
            flow_key,
        };
        if let Some(url) = &settings.discovery {
            settings.validate_endpoint(url)?;
        }
        Ok(settings)
    }

    fn validate_endpoint(&self, url: &Url) -> Result<()> {
        validate_transport(url)?;
        ensure!(
            self.allowed_origins
                .contains(&url.origin().ascii_serialization()),
            "endpoint origin is not allowed"
        );
        Ok(())
    }

    fn client(&self, metadata: CoreProviderMetadata) -> Result<Client> {
        Ok(CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(self.client_id.clone()),
            Some(ClientSecret::new(self.client_secret.as_str().to_owned())),
        )
        .set_redirect_uri(RedirectUrl::new(self.callback.to_string())?)
        .set_auth_type(self.auth_type.clone()))
    }
}

fn fingerprint(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.len().to_be_bytes());
        hash.update(part.as_bytes());
    }
    hex::encode(hash.finalize())
}
fn validate_transport(url: &Url) -> Result<()> {
    ensure!(url.scheme() == "https", "HTTPS required");
    ensure!(
        url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "invalid endpoint URL"
    );
    Ok(())
}

struct Cache {
    metadata: Option<CoreProviderMetadata>,
    last_discovery: Option<Instant>,
    last_keys: Option<Instant>,
}

pub(crate) struct Runtime {
    pub revision: i64,
    pub configuration: Configuration,
    pub enabled: bool,
    pub label: String,
    pub settings: Option<Settings>,
    http: Option<reqwest::Client>,
    cache: Mutex<Cache>,
    pub verification: tokio::sync::Semaphore,
    pub logout_verification: tokio::sync::Semaphore,
}

impl Runtime {
    pub fn new(config: &Config, configuration: Configuration) -> Self {
        let enabled = configuration.enabled;
        let label = configuration.label.clone();
        let mut settings = if enabled {
            match Settings::from_configuration(config, &configuration) {
                Ok(settings) => Some(settings),
                Err(_) => {
                    tracing::error!("OIDC configuration invalid; local login remains available");
                    None
                }
            }
        } else {
            None
        };
        let mut http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8));
        if let Some(certificate) = settings.as_ref().and_then(|s| s.ca_certificate.clone()) {
            http = http.add_root_certificate(certificate);
        }
        let http = match http.build() {
            Ok(http) => Some(http),
            Err(_) => {
                tracing::error!("OIDC HTTP configuration invalid; local login remains available");
                settings = None;
                None
            }
        };
        Self {
            revision: 0,
            configuration,
            enabled,
            label,
            settings,
            http,
            cache: Mutex::new(Cache {
                metadata: None,
                last_discovery: None,
                last_keys: None,
            }),
            verification: tokio::sync::Semaphore::new(4),
            logout_verification: tokio::sync::Semaphore::new(4),
        }
    }

    pub async fn apply(&self, tx: &mut Transaction<'_, Postgres>) -> Result<()> {
        // Configuration and OIDC mutations always lock this row before locking users.
        let (previous_identity, previous_flow) =
            sqlx::query_as::<_, (Option<String>, Option<String>)>(
                "SELECT identity_key, flow_key FROM oidc_configuration WHERE singleton FOR UPDATE",
            )
            .fetch_one(&mut **tx)
            .await?;
        sqlx::query("SELECT id FROM users ORDER BY id FOR UPDATE")
            .execute(&mut **tx)
            .await?;
        let identity = self.settings.as_ref().map(|s| &s.identity_key);
        let flow = self.settings.as_ref().map(|s| &s.flow_key);
        if identity.is_none() || previous_identity.as_ref() != identity {
            sqlx::query("DELETE FROM sessions WHERE authentication_method = 'oidc'")
                .execute(&mut **tx)
                .await?;
        }
        if flow.is_none() || previous_flow.as_ref() != flow {
            sqlx::query("DELETE FROM oidc_login_attempts")
                .execute(&mut **tx)
                .await?;
        }
        if let Some(settings) = &self.settings {
            sqlx::query("UPDATE sessions SET expires_at = LEAST(expires_at, created_at + make_interval(secs => $1)) WHERE authentication_method = 'oidc'")
                .bind(settings.max_age as f64).execute(&mut **tx).await?;
        }
        if let Some(identity) = identity {
            sqlx::query("UPDATE oidc_identities SET needs_relink = true, updated_at = now() WHERE configuration_key <> $1")
                .bind(identity).execute(&mut **tx).await?;
        }
        sqlx::query(
            "UPDATE oidc_configuration SET identity_key = $1, flow_key = $2 WHERE singleton",
        )
        .bind(identity)
        .bind(flow)
        .execute(&mut **tx)
        .await?;
        // Discovery is lazy and retried at most once per minute; local requests never await it.
        Ok(())
    }

    pub fn settings(&self) -> Result<&Settings> {
        self.settings.as_ref().context("OIDC unavailable")
    }

    async fn request(&self, mut request: HttpRequest) -> Result<HttpResponse, std::io::Error> {
        let result: Result<_> = async {
            let settings = self.settings()?;
            let default_discovery = format!(
                "{}/.well-known/openid-configuration",
                settings.issuer.trim_end_matches('/')
            );
            if request.uri().to_string() == default_discovery
                && let Some(url) = &settings.discovery
            {
                *request.uri_mut() = url.as_str().parse()?;
            }
            let url = Url::parse(&request.uri().to_string())?;
            settings.validate_endpoint(&url)?;
            let (parts, body) = request.into_parts();
            let mut response = self
                .http
                .as_ref()
                .context("OIDC HTTP unavailable")?
                .request(parts.method, url)
                .headers(parts.headers)
                .body(body)
                .send()
                .await?;
            ensure!(!response.status().is_redirection(), "redirect refused");
            ensure!(
                response
                    .content_length()
                    .is_none_or(|n| n <= RESPONSE_LIMIT as u64),
                "response too large"
            );
            let mut builder = axum::http::Response::builder().status(response.status());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    body.len() + chunk.len() <= RESPONSE_LIMIT,
                    "response too large"
                );
                body.extend_from_slice(&chunk);
            }
            Ok(builder.body(body)?)
        }
        .await;
        // HTTP errors may contain URLs, codes or response bodies. Never propagate those to logs.
        result.map_err(|_| std::io::Error::other("OIDC HTTP request failed"))
    }

    pub async fn metadata(&self) -> Result<CoreProviderMetadata> {
        let settings = self.settings()?;
        let mut cache = self.cache.lock().await;
        if cache
            .last_discovery
            .is_some_and(|t| t.elapsed() < METADATA_MAX_AGE)
            && let Some(metadata) = &cache.metadata
        {
            return Ok(metadata.clone());
        }
        ensure!(
            cache
                .last_discovery
                .is_none_or(|t| t.elapsed() >= RETRY_INTERVAL),
            "discovery cooling down"
        );
        // Discard expired keys before fetching so failures cannot extend their trust.
        cache.metadata = None;
        cache.last_discovery = Some(Instant::now());
        let metadata =
            CoreProviderMetadata::discover_async(IssuerUrl::new(settings.issuer.clone())?, self)
                .await
                .map_err(|_| anyhow::anyhow!("OIDC discovery failed"))?;
        ensure!(
            metadata.issuer().as_str() == settings.issuer,
            "issuer mismatch"
        );
        settings.validate_endpoint(metadata.authorization_endpoint().url())?;
        settings.validate_endpoint(
            metadata
                .token_endpoint()
                .context("missing token endpoint")?
                .url(),
        )?;
        settings.validate_endpoint(metadata.jwks_uri().url())?;
        ensure!(
            metadata
                .id_token_signing_alg_values_supported()
                .contains(&CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256),
            "RS256 required"
        );
        if let Some(methods) = metadata.token_endpoint_auth_methods_supported() {
            let expected = if matches!(settings.auth_type, AuthType::BasicAuth) {
                "client_secret_basic"
            } else {
                "client_secret_post"
            };
            ensure!(
                methods
                    .iter()
                    .any(|m| serde_json::to_value(m).is_ok_and(|v| v == expected)),
                "unsupported token authentication"
            );
        }
        cache.metadata = Some(metadata.clone());
        Ok(metadata)
    }

    async fn metadata_for_token(&self, token: &str) -> Result<CoreProviderMetadata> {
        let header = decode_header(token)?;
        ensure!(header.alg == Algorithm::RS256, "RS256 required");
        let metadata = self.metadata().await?;
        let known = |metadata: &CoreProviderMetadata| -> Result<bool> {
            let keys: JwkSet = serde_json::from_value(serde_json::to_value(metadata.jwks())?)?;
            Ok(keys
                .keys
                .iter()
                .any(|key| header.kid.is_none() || key.common.key_id == header.kid))
        };
        if known(&metadata)? {
            return Ok(metadata);
        }
        let mut cache = self.cache.lock().await;
        let current = cache.metadata.as_ref().context("metadata unavailable")?;
        if known(current)? {
            return Ok(current.clone());
        }
        ensure!(
            cache
                .last_keys
                .is_none_or(|t| t.elapsed() >= RETRY_INTERVAL),
            "key refresh cooling down"
        );
        let metadata = current.clone();
        let uri = metadata.jwks_uri().clone();
        cache.last_keys = Some(Instant::now());
        let keys = CoreJsonWebKeySet::fetch_async(&uri, self)
            .await
            .map_err(|_| anyhow::anyhow!("key refresh failed"))?;
        let metadata = metadata.set_jwks(keys);
        cache.metadata = Some(metadata.clone());
        Ok(metadata)
    }

    pub async fn authorization(&self) -> Result<(String, String, String, String)> {
        let client = self.settings()?.client(self.metadata().await?)?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge)
            .add_extra_param("response_mode", "query")
            .url();
        Ok((
            url.to_string(),
            state.secret().clone(),
            nonce.secret().clone(),
            verifier.secret().clone(),
        ))
    }

    pub async fn exchange(
        &self,
        code: String,
        verifier: String,
        nonce: String,
    ) -> Result<Identity> {
        let settings = self.settings()?;
        let client = settings.client(self.metadata().await?)?;
        let response = client
            .exchange_code(AuthorizationCode::new(code))?
            .set_pkce_verifier(PkceCodeVerifier::new(verifier))
            .request_async(self)
            .await
            .map_err(|_| {
                tracing::warn!("OIDC token exchange rejected or returned an invalid response");
                anyhow::anyhow!("OIDC code exchange failed")
            })?;
        let token = response
            .id_token()
            .ok_or_else(|| {
                tracing::warn!("OIDC token response omitted the ID token");
                anyhow::anyhow!("missing ID token")
            })?
            .to_string();
        let metadata = self.metadata_for_token(&token).await?;
        let client = settings.client(metadata)?;
        let verifier = client
            .id_token_verifier()
            .set_allowed_algs([CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256])
            .set_issue_time_verifier_fn(|iat| {
                if iat > Utc::now() + chrono::Duration::seconds(60) {
                    Err("future ID token".to_owned())
                } else {
                    Ok(())
                }
            });
        let token: IdToken<
            ExtraClaims,
            CoreGenderClaim,
            CoreJweContentEncryptionAlgorithm,
            CoreJwsSigningAlgorithm,
        > = token.parse()?;
        let claims = token.claims(&verifier, &Nonce::new(nonce)).map_err(|_| {
            tracing::warn!("OIDC ID token signature or claims validation failed");
            anyhow::anyhow!("invalid ID token")
        })?;
        ensure!(
            claims.issuer().as_str() == settings.issuer,
            "issuer mismatch"
        );
        ensure!(
            !claims.subject().as_str().is_empty() && claims.subject().as_str().len() <= 255,
            "invalid subject"
        );
        let extra = claims.additional_claims();
        ensure!(
            extra.nbf.is_none_or(|n| n <= Utc::now().timestamp() + 60),
            "token not yet valid"
        );
        ensure!(
            extra
                .sid
                .as_ref()
                .is_none_or(|sid| !sid.is_empty() && sid.len() <= 1024),
            "invalid session ID"
        );
        Ok(Identity {
            subject: claims.subject().to_string(),
            sid: extra.sid.clone(),
        })
    }

    pub async fn logout_claims(&self, token: &str) -> Result<LogoutClaims> {
        let settings = self.settings()?;
        let metadata = self.metadata_for_token(token).await?;
        let header = decode_header(token)?;
        let keys: JwkSet = serde_json::from_value(serde_json::to_value(metadata.jwks())?)?;
        let candidates: Vec<_> = keys
            .keys
            .iter()
            .filter(|key| header.kid.is_none() || key.common.key_id == header.kid)
            .collect();
        ensure!(candidates.len() == 1, "ambiguous signing key");
        let selected = candidates[0];
        ensure!(
            selected
                .common
                .public_key_use
                .as_ref()
                .is_none_or(|u| *u == PublicKeyUse::Signature)
                && selected
                    .common
                    .key_algorithm
                    .is_none_or(|a| a == KeyAlgorithm::RS256)
                && selected
                    .common
                    .key_operations
                    .as_ref()
                    .is_none_or(|ops| ops.contains(&KeyOperations::Verify)),
            "key not allowed for signature verification"
        );
        let key = DecodingKey::from_jwk(selected)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_required_spec_claims(&["iss", "aud", "iat", "jti"]);
        validation.set_issuer(&[&settings.issuer]);
        validation.set_audience(&[&settings.client_id]);
        validation.validate_nbf = true;
        validation.leeway = 60;
        let claims = decode::<LogoutClaims>(token, &key, &validation)?.claims;
        claims.validate(Utc::now())?;
        Ok(claims)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ExtraClaims {
    sid: Option<String>,
    nbf: Option<i64>,
}
impl openidconnect::AdditionalClaims for ExtraClaims {}
pub(crate) struct Identity {
    pub subject: String,
    pub sid: Option<String>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct LogoutClaims {
    pub iss: String,
    pub jti: String,
    pub sid: Option<String>,
    pub sub: Option<String>,
    iat: i64,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}
impl LogoutClaims {
    fn validate(&self, now: DateTime<Utc>) -> Result<()> {
        ensure!(
            self.iat >= now.timestamp() - 600 && self.iat <= now.timestamp() + 60,
            "stale logout token"
        );
        ensure!(
            !self.jti.is_empty() && self.jti.len() <= 1024,
            "invalid JWT ID"
        );
        ensure!(!self.other.contains_key("nonce"), "logout nonce forbidden");
        ensure!(
            self.sid.is_some() || self.sub.is_some(),
            "missing logout target"
        );
        ensure!(
            self.sid
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 1024)
                && self
                    .sub
                    .as_ref()
                    .is_none_or(|s| !s.is_empty() && s.len() <= 255),
            "invalid logout target"
        );
        ensure!(
            self.other
                .get("events")
                .and_then(|e| e.get("http://schemas.openid.net/event/backchannel-logout"))
                .is_some_and(|v| v.as_object().is_some_and(|o| o.is_empty())),
            "missing logout event"
        );
        Ok(())
    }
}

impl<'c> openidconnect::AsyncHttpClient<'c> for Runtime {
    type Error = std::io::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, Self::Error>> + Send + 'c>,
    >;
    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(self.request(request))
    }
}
