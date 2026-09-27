use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    source_key: String,
    description: String,
    tags: Vec<String>,
    adapter: String,
    base_url: String,
    official_url: String,
    #[serde(default)]
    page_id: Option<String>,
    #[serde(default)]
    separate_incidents_endpoint: bool,
    #[serde(default)]
    summary_path: Option<String>,
    #[serde(default)]
    components_path: Option<String>,
    #[serde(default)]
    maintenance_path: Option<String>,
    #[serde(default)]
    incidents_path: Option<String>,
    #[serde(default)]
    native_incidents_path: Option<String>,
    #[serde(default)]
    component_scope: ComponentScope,
    #[serde(default)]
    component_name_prefixes: Vec<String>,
    #[serde(default)]
    excluded_incident_name_prefixes: Vec<String>,
    providers: Vec<CatalogProvider>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ComponentScope {
    #[default]
    All,
    Listed,
}

#[derive(Debug, Deserialize)]
struct CatalogProvider {
    slug: String,
    name: String,
    components: Vec<CatalogComponent>,
}

#[derive(Debug, Deserialize)]
struct CatalogComponent {
    upstream_id: String,
    name: String,
    group: Option<String>,
    position: i32,
}

const CATALOGS: &[(&str, &str)] = &[
    ("console.yaml", include_str!("console.yaml")),
    ("intercom.yaml", include_str!("intercom.yaml")),
    ("gong.yaml", include_str!("gong.yaml")),
    ("gainsight.yaml", include_str!("gainsight.yaml")),
    ("digitalocean.yaml", include_str!("digitalocean.yaml")),
    ("airtable.yaml", include_str!("airtable.yaml")),
    ("avalara.yaml", include_str!("avalara.yaml")),
    ("aws.yaml", include_str!("aws.yaml")),
    ("jira.yaml", include_str!("jira.yaml")),
    ("jsm.yaml", include_str!("jsm.yaml")),
    ("asana.yaml", include_str!("asana.yaml")),
    ("ashby.yaml", include_str!("ashby.yaml")),
    ("bitbucket.yaml", include_str!("bitbucket.yaml")),
    ("circleci.yaml", include_str!("circleci.yaml")),
    ("claude.yaml", include_str!("claude.yaml")),
    ("cloudflare.yaml", include_str!("cloudflare.yaml")),
    ("confluence.yaml", include_str!("confluence.yaml")),
    ("datadog.yaml", include_str!("datadog.yaml")),
    ("discord.yaml", include_str!("discord.yaml")),
    ("docker.yaml", include_str!("docker.yaml")),
    ("dropbox.yaml", include_str!("dropbox.yaml")),
    ("figma.yaml", include_str!("figma.yaml")),
    ("gemini.yaml", include_str!("gemini.yaml")),
    ("github.yaml", include_str!("github.yaml")),
    ("gitlab.yaml", include_str!("gitlab.yaml")),
    ("google_cloud.yaml", include_str!("google_cloud.yaml")),
    ("grafana.yaml", include_str!("grafana.yaml")),
    ("hubspot.yaml", include_str!("hubspot.yaml")),
    ("launchdarkly.yaml", include_str!("launchdarkly.yaml")),
    ("mailgun.yaml", include_str!("mailgun.yaml")),
    ("marketo.yaml", include_str!("marketo.yaml")),
    ("mongodb.yaml", include_str!("mongodb.yaml")),
    ("netsuite.yaml", include_str!("netsuite.yaml")),
    ("notion.yaml", include_str!("notion.yaml")),
    ("okta.yaml", include_str!("okta.yaml")),
    ("onepassword.yaml", include_str!("onepassword.yaml")),
    ("openai.yaml", include_str!("openai.yaml")),
    ("pagerduty.yaml", include_str!("pagerduty.yaml")),
    ("postman.yaml", include_str!("postman.yaml")),
    ("rippling.yaml", include_str!("rippling.yaml")),
    ("salesforce.yaml", include_str!("salesforce.yaml")),
    ("sigma.yaml", include_str!("sigma.yaml")),
    ("slack.yaml", include_str!("slack.yaml")),
    ("sentry.yaml", include_str!("sentry.yaml")),
    ("shopify.yaml", include_str!("shopify.yaml")),
    ("snowflake.yaml", include_str!("snowflake.yaml")),
    ("stripe.yaml", include_str!("stripe.yaml")),
    ("supabase.yaml", include_str!("supabase.yaml")),
    ("trello.yaml", include_str!("trello.yaml")),
    ("twilio.yaml", include_str!("twilio.yaml")),
    ("vercel.yaml", include_str!("vercel.yaml")),
    ("workato.yaml", include_str!("workato.yaml")),
    ("zapier.yaml", include_str!("zapier.yaml")),
    ("zoom.yaml", include_str!("zoom.yaml")),
];

// Tags are user-visible exact-match filters, so keep their shared vocabulary centralized.
const APPROVED_TAGS: &[&str] = &[
    "AI",
    "API",
    "Analytics",
    "Automation",
    "Business operations",
    "CI/CD",
    "Cloud",
    "Collaboration",
    "Commerce",
    "Communications",
    "CRM",
    "Data",
    "Database",
    "Developer tools",
    "DevOps",
    "Finance",
    "HR",
    "Identity & access",
    "Infrastructure",
    "Integration",
    "IT operations",
    "Knowledge base",
    "Marketing",
    "Observability",
    "Payments",
    "Project management",
    "Security",
    "Source control",
    "Support",
    "Workflow",
];

const APPROVED_SOURCES: &[(&str, &str, &str, &str)] = &[
    (
        "console",
        "statuspage",
        "https://status.console.com",
        "https://status.console.com",
    ),
    (
        "intercom",
        "intercom",
        "https://www.finstatus.com",
        "https://www.finstatus.com",
    ),
    (
        "gong",
        "statuspage",
        "https://status.gong.io",
        "https://status.gong.io",
    ),
    (
        "gainsight",
        "statuspage",
        "https://status.gainsight.com",
        "https://status.gainsight.com",
    ),
    (
        "digitalocean",
        "statuspage",
        "https://status.digitalocean.com",
        "https://status.digitalocean.com",
    ),
    (
        "airtable",
        "statuspage",
        "https://status.airtable.com",
        "https://status.airtable.com",
    ),
    (
        "avalara",
        "statuspage",
        "https://status.avalara.com",
        "https://status.avalara.com",
    ),
    (
        "aws",
        "aws",
        "https://health.aws.amazon.com",
        "https://health.aws.amazon.com",
    ),
    (
        "jira",
        "statuspage",
        "https://jira-software.status.atlassian.com",
        "https://jira-software.status.atlassian.com",
    ),
    (
        "jsm",
        "statuspage",
        "https://jira-service-management.status.atlassian.com",
        "https://jira-service-management.status.atlassian.com",
    ),
    (
        "asana",
        "statuspage",
        "https://status.asana.com",
        "https://status.asana.com",
    ),
    (
        "ashby",
        "statuspage",
        "https://status.ashbyhq.com",
        "https://status.ashbyhq.com",
    ),
    (
        "bitbucket",
        "statuspage",
        "https://bitbucket.status.atlassian.com",
        "https://bitbucket.status.atlassian.com",
    ),
    (
        "circleci",
        "statuspage",
        "https://status.circleci.com",
        "https://status.circleci.com",
    ),
    (
        "claude",
        "statuspage",
        "https://status.claude.com",
        "https://status.claude.com",
    ),
    (
        "cloudflare",
        "statuspage",
        "https://www.cloudflarestatus.com",
        "https://www.cloudflarestatus.com",
    ),
    (
        "confluence",
        "statuspage",
        "https://confluence.status.atlassian.com",
        "https://confluence.status.atlassian.com",
    ),
    (
        "github",
        "statuspage",
        "https://www.githubstatus.com",
        "https://www.githubstatus.com",
    ),
    (
        "datadog",
        "datadog",
        "https://status.datadoghq.com",
        "https://status.datadoghq.com",
    ),
    (
        "discord",
        "statuspage",
        "https://discordstatus.com",
        "https://discordstatus.com",
    ),
    (
        "docker",
        "statuspage",
        "https://www.dockerstatus.com",
        "https://www.dockerstatus.com",
    ),
    (
        "dropbox",
        "statuspage",
        "https://status.dropbox.com",
        "https://status.dropbox.com",
    ),
    (
        "figma",
        "statuspage",
        "https://status.figma.com",
        "https://status.figma.com",
    ),
    (
        "gemini",
        "google_cloud",
        "https://status.cloud.google.com",
        "https://status.cloud.google.com",
    ),
    (
        "gitlab",
        "statusio",
        "https://status.gitlab.com",
        "https://status.gitlab.com",
    ),
    (
        "google_cloud",
        "google_cloud",
        "https://status.cloud.google.com",
        "https://status.cloud.google.com",
    ),
    (
        "grafana",
        "statuspage",
        "https://status.grafana.com",
        "https://status.grafana.com",
    ),
    (
        "hubspot",
        "statuspage",
        "https://status.hubspot.com",
        "https://status.hubspot.com",
    ),
    (
        "launchdarkly",
        "statuspage",
        "https://status.launchdarkly.com",
        "https://status.launchdarkly.com",
    ),
    (
        "mailgun",
        "statuspage",
        "https://status.mailgun.com",
        "https://status.mailgun.com",
    ),
    (
        "marketo",
        "adobe",
        "https://data.status.adobe.com",
        "https://status.adobe.com",
    ),
    (
        "mongodb",
        "statuspage",
        "https://status.mongodb.com",
        "https://status.mongodb.com",
    ),
    (
        "netsuite",
        "statuspage",
        "https://status.netsuite.com",
        "https://status.netsuite.com",
    ),
    (
        "notion",
        "statuspage",
        "https://www.notion-status.com",
        "https://www.notion-status.com",
    ),
    (
        "okta",
        "okta",
        "https://status.okta.com",
        "https://status.okta.com",
    ),
    (
        "onepassword",
        "statuspage",
        "https://status.1password.com",
        "https://status.1password.com",
    ),
    (
        "openai",
        "statuspage",
        "https://status.openai.com",
        "https://status.openai.com",
    ),
    (
        "pagerduty",
        "pagerduty",
        "https://status.pagerduty.com",
        "https://status.pagerduty.com",
    ),
    (
        "postman",
        "statuspage",
        "https://status.postman.com",
        "https://status.postman.com",
    ),
    (
        "rippling",
        "statuspage",
        "https://status.rippling.com",
        "https://status.rippling.com",
    ),
    (
        "salesforce",
        "salesforce",
        "https://api.status.salesforce.com",
        "https://status.salesforce.com",
    ),
    (
        "sigma",
        "statuspage",
        "https://status.sigmacomputing.com",
        "https://status.sigmacomputing.com",
    ),
    (
        "slack",
        "slack",
        "https://slack-status.com",
        "https://slack-status.com",
    ),
    (
        "sentry",
        "statuspage",
        "https://t687h3m0nh65.statuspage.io",
        "https://status.sentry.io",
    ),
    (
        "shopify",
        "statuspage",
        "https://www.shopifystatus.com",
        "https://www.shopifystatus.com",
    ),
    (
        "snowflake",
        "statuspage",
        "https://status.snowflake.com",
        "https://status.snowflake.com",
    ),
    (
        "stripe",
        "statuspage",
        "https://www.stripestatus.com",
        "https://status.stripe.com",
    ),
    (
        "supabase",
        "statuspage",
        "https://status.supabase.com",
        "https://status.supabase.com",
    ),
    (
        "trello",
        "statuspage",
        "https://trello.status.atlassian.com",
        "https://trello.status.atlassian.com",
    ),
    (
        "twilio",
        "statuspage",
        "https://status.twilio.com",
        "https://status.twilio.com",
    ),
    (
        "vercel",
        "statuspage",
        "https://www.vercel-status.com",
        "https://www.vercel-status.com",
    ),
    (
        "workato",
        "statuspage",
        "https://status.workato.com",
        "https://status.workato.com",
    ),
    (
        "zapier",
        "statuspage",
        "https://status.zapier.com",
        "https://status.zapier.com",
    ),
    (
        "zoom",
        "statuspage",
        "https://www.zoomstatus.com",
        "https://www.zoomstatus.com",
    ),
];

pub fn validate() -> anyhow::Result<()> {
    let mut source_keys = HashSet::new();
    let mut tag_counts = HashMap::new();
    for (filename, content) in CATALOGS {
        let catalog: Catalog =
            serde_yaml_ng::from_str(content).with_context(|| format!("invalid {filename}"))?;
        validate_catalog(&catalog).with_context(|| format!("invalid {filename}"))?;
        if !source_keys.insert(catalog.source_key.clone()) {
            bail!("duplicate catalog source key {}", catalog.source_key);
        }
        for tag in &catalog.tags {
            *tag_counts.entry(tag.clone()).or_insert(0_usize) += 1;
        }
    }
    let underused_tags = APPROVED_TAGS
        .iter()
        .filter_map(|tag| {
            let count = tag_counts.get(*tag).copied().unwrap_or_default();
            (count < 2).then(|| format!("{tag} ({count})"))
        })
        .collect::<Vec<_>>();
    if !underused_tags.is_empty() {
        bail!(
            "approved catalog tags must be assigned to at least two providers: {}",
            underused_tags.join(", ")
        );
    }
    Ok(())
}

pub async fn reconcile(pool: &PgPool, poll_interval: Duration) -> Result<()> {
    validate()?;
    let mut active_source_keys = Vec::with_capacity(CATALOGS.len());
    for (_, content) in CATALOGS {
        let catalog: Catalog = serde_yaml_ng::from_str(content)?;
        active_source_keys.push(catalog.source_key.clone());
        let mut tx = pool.begin().await?;
        let source_id = upsert_source(&mut tx, &catalog, poll_interval).await?;
        let provider_keys = catalog
            .providers
            .iter()
            .map(|provider| provider.slug.clone())
            .collect::<Vec<_>>();
        for provider in catalog.providers {
            let provider_id = sqlx::query_scalar::<_, Uuid>("
                INSERT INTO providers (provider_source_id, slug, name, description, tags, upstream_provider_key, official_url, active)
                VALUES ($1, $2, $3, $4, $5, $2, $6, true)
                ON CONFLICT (provider_source_id, upstream_provider_key)
                DO UPDATE SET slug = EXCLUDED.slug, name = EXCLUDED.name, description = EXCLUDED.description, tags = EXCLUDED.tags, official_url = EXCLUDED.official_url, active = true
                RETURNING id
            ")
            .bind(source_id)
            .bind(&provider.slug)
            .bind(&provider.name)
            .bind(&catalog.description)
            .bind(&catalog.tags)
            .bind(&catalog.official_url)
            .fetch_one(&mut *tx)
            .await?;
            for component in provider.components {
                sqlx::query("
                    INSERT INTO components (provider_id, upstream_component_id, name, group_name, position, active)
                    VALUES ($1, $2, $3, $4, $5, true)
                    ON CONFLICT (provider_id, upstream_component_id)
                    DO UPDATE SET name = EXCLUDED.name, group_name = EXCLUDED.group_name, position = EXCLUDED.position, active = true
                ")
                .bind(provider_id)
                .bind(&component.upstream_id)
                .bind(&component.name)
                .bind(&component.group)
                .bind(component.position)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query("UPDATE providers SET active = false WHERE provider_source_id = $1 AND NOT (upstream_provider_key = ANY($2))")
            .bind(source_id)
            .bind(&provider_keys)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE components SET active = false WHERE provider_id IN (SELECT id FROM providers WHERE provider_source_id = $1 AND NOT active)")
            .bind(source_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO audit_log (action, entity_type, entity_id, metadata) VALUES ('catalog.reconciled', 'provider_source', $1, jsonb_build_object('provider_count', $2::integer))")
            .bind(source_id)
            .bind(i32::try_from(provider_keys.len()).context("catalog has too many providers")?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    let mut tx = pool.begin().await?;
    let deactivated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE provider_sources SET enabled = false, lease_owner = NULL, lease_until = NULL WHERE enabled AND NOT (source_key = ANY($1)) RETURNING id",
    )
    .bind(&active_source_keys)
    .fetch_all(&mut *tx)
    .await?;
    if !deactivated.is_empty() {
        sqlx::query("UPDATE providers SET active = false WHERE provider_source_id = ANY($1)")
            .bind(&deactivated)
            .execute(&mut *tx)
            .await?;
        for source_id in deactivated {
            sqlx::query("INSERT INTO audit_log (action, entity_type, entity_id) VALUES ('catalog.source_deactivated', 'provider_source', $1)")
                .bind(source_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn upsert_source(
    tx: &mut Transaction<'_, Postgres>,
    catalog: &Catalog,
    poll_interval: Duration,
) -> Result<Uuid> {
    let mut public_config = json!({"base_url": catalog.base_url});
    if catalog.component_scope == ComponentScope::Listed {
        public_config["component_ids"] = json!(
            catalog
                .providers
                .iter()
                .flat_map(|provider| &provider.components)
                .map(|component| &component.upstream_id)
                .collect::<Vec<_>>()
        );
    }
    if !catalog.component_name_prefixes.is_empty() {
        public_config["component_name_prefixes"] = json!(&catalog.component_name_prefixes);
    }
    if !catalog.excluded_incident_name_prefixes.is_empty() {
        public_config["excluded_incident_name_prefixes"] =
            json!(&catalog.excluded_incident_name_prefixes);
    }
    if let Some(page_id) = &catalog.page_id {
        public_config["page_id"] = json!(page_id);
    }
    if catalog.separate_incidents_endpoint {
        public_config["separate_incidents_endpoint"] = json!(true);
    }
    if let Some(summary_path) = &catalog.summary_path {
        public_config["summary_path"] = json!(summary_path);
    }
    if let Some(components_path) = &catalog.components_path {
        public_config["components_path"] = json!(components_path);
    }
    if let Some(maintenance_path) = &catalog.maintenance_path {
        public_config["maintenance_path"] = json!(maintenance_path);
    }
    if let Some(incidents_path) = &catalog.incidents_path {
        public_config["incidents_path"] = json!(incidents_path);
    }
    if let Some(native_incidents_path) = &catalog.native_incidents_path {
        public_config["native_incidents_path"] = json!(native_incidents_path);
    }
    // A full poll reconciles catalog changes and refreshes discovered component metadata.
    Ok(sqlx::query_scalar::<_, Uuid>("
        INSERT INTO provider_sources (source_key, adapter, base_url, public_config, enabled, poll_interval_seconds, next_poll_at)
        VALUES ($1, $2, $3, $4, true, $5, now())
        ON CONFLICT (source_key)
        DO UPDATE SET adapter = EXCLUDED.adapter, base_url = EXCLUDED.base_url,
                      public_config = EXCLUDED.public_config, enabled = true,
                      history_refreshed_at = CASE WHEN provider_sources.public_config IS DISTINCT FROM EXCLUDED.public_config
                          OR provider_sources.adapter <> EXCLUDED.adapter THEN NULL ELSE provider_sources.history_refreshed_at END,
                      history_next_poll_at = CASE WHEN provider_sources.public_config IS DISTINCT FROM EXCLUDED.public_config
                          OR provider_sources.adapter <> EXCLUDED.adapter THEN now() ELSE provider_sources.history_next_poll_at END,
                      history_consecutive_failures = CASE WHEN provider_sources.public_config IS DISTINCT FROM EXCLUDED.public_config
                          OR provider_sources.adapter <> EXCLUDED.adapter THEN 0 ELSE provider_sources.history_consecutive_failures END,
                      history_last_error = CASE WHEN provider_sources.public_config IS DISTINCT FROM EXCLUDED.public_config
                          OR provider_sources.adapter <> EXCLUDED.adapter THEN NULL ELSE provider_sources.history_last_error END,
                      poll_interval_seconds = EXCLUDED.poll_interval_seconds,
                      etag = NULL, last_modified = NULL, next_poll_at = now()
        RETURNING id
    ")
    .bind(&catalog.source_key)
    .bind(&catalog.adapter)
    .bind(&catalog.base_url)
    .bind(public_config)
    .bind(i32::try_from(poll_interval.as_secs()).context("poll interval is too large")?)
    .fetch_one(&mut **tx)
    .await?)
}

fn validate_catalog(catalog: &Catalog) -> Result<()> {
    if catalog.description.trim().is_empty() || catalog.description.len() > 240 {
        bail!("catalog description must contain between 1 and 240 characters");
    }
    if !(3..=5).contains(&catalog.tags.len()) {
        bail!("catalog must define between 3 and 5 tags");
    }
    let mut tags = HashSet::new();
    for tag in &catalog.tags {
        if tag.trim().is_empty() || tag.len() > 40 || !tags.insert(tag) {
            bail!("catalog tags must be unique and contain between 1 and 40 characters");
        }
        if !APPROVED_TAGS.contains(&tag.as_str()) {
            bail!("catalog tag {tag:?} is not approved");
        }
    }
    if !catalog
        .tags
        .windows(2)
        .all(|pair| pair[0].to_ascii_lowercase() <= pair[1].to_ascii_lowercase())
    {
        bail!("catalog tags must be alphabetized");
    }
    let base = Url::parse(&catalog.base_url)?;
    let official = Url::parse(&catalog.official_url)?;
    if [base, official].into_iter().any(|url| {
        url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
    }) {
        bail!("catalog URLs must be credential-free HTTPS origins");
    }
    let approved = APPROVED_SOURCES
        .iter()
        .find(|(source_key, _, _, _)| *source_key == catalog.source_key);
    let Some((_, adapter, base_url, official_url)) = approved else {
        bail!(
            "catalog source {} is not approved for V1",
            catalog.source_key
        );
    };
    if catalog.adapter != *adapter
        || catalog.base_url != *base_url
        || catalog.official_url != *official_url
    {
        bail!(
            "catalog source {} does not match its approved adapter and origins",
            catalog.source_key
        );
    }
    if catalog.component_scope == ComponentScope::Listed
        && !matches!(
            catalog.adapter.as_str(),
            "statuspage" | "statusio" | "pagerduty" | "google_cloud" | "salesforce" | "aws"
        )
    {
        bail!("listed component scope is not supported by this adapter");
    }
    if (catalog.adapter == "statusio") != catalog.page_id.is_some() {
        bail!("page_id is required only for the statusio adapter");
    }
    if catalog.page_id.as_ref().is_some_and(|page_id| {
        page_id.is_empty() || !page_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    }) {
        bail!("page_id must contain only ASCII letters and numbers");
    }
    if catalog.separate_incidents_endpoint && catalog.adapter != "statuspage" {
        bail!("separate incidents require the statuspage adapter");
    }
    for (name, path) in [
        ("summary_path", &catalog.summary_path),
        ("components_path", &catalog.components_path),
        ("maintenance_path", &catalog.maintenance_path),
        ("incidents_path", &catalog.incidents_path),
        ("native_incidents_path", &catalog.native_incidents_path),
    ] {
        if let Some(path) = path {
            if catalog.adapter != "statuspage" {
                bail!("{name} requires the statuspage adapter");
            }
            if !path.starts_with('/') || path.starts_with("//") || path.contains(['?', '#']) {
                bail!("{name} must be an absolute URL path without a query or fragment");
            }
        }
    }
    if !catalog.component_name_prefixes.is_empty() {
        if catalog.adapter != "statuspage" {
            bail!("component name prefixes require the statuspage adapter");
        }
        if catalog.component_scope == ComponentScope::Listed {
            bail!("component IDs and name prefixes cannot be combined");
        }
        let mut prefixes = HashSet::new();
        for prefix in &catalog.component_name_prefixes {
            if prefix.is_empty() {
                bail!("component name prefixes must not be empty");
            }
            if !prefixes.insert(prefix) {
                bail!("duplicate component name prefix {prefix}");
            }
        }
    }
    if !catalog.excluded_incident_name_prefixes.is_empty() {
        if catalog.adapter != "statuspage" {
            bail!("incident name exclusions require the statuspage adapter");
        }
        let mut prefixes = HashSet::new();
        for prefix in &catalog.excluded_incident_name_prefixes {
            if prefix.is_empty() {
                bail!("incident name exclusions must not be empty");
            }
            if !prefixes.insert(prefix) {
                bail!("duplicate incident name exclusion {prefix}");
            }
        }
    }
    if catalog.providers.is_empty() {
        bail!("catalog source must contain a provider");
    }
    let mut slugs = HashSet::new();
    for provider in &catalog.providers {
        if provider.components.is_empty() {
            bail!("provider {} must define components", provider.slug);
        }
        if !slugs.insert(&provider.slug) {
            bail!("duplicate provider slug {}", provider.slug);
        }
        let mut ids = HashSet::new();
        for component in &provider.components {
            if !ids.insert(&component.upstream_id) {
                bail!("duplicate component {}", component.upstream_id);
            }
        }
    }
    Ok(())
}
