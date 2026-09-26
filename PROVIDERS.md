# Supported providers

StatusDeck currently ships with curated support for the following provider
status sources. Displayed hostnames omit a leading `www.` for consistency,
while links preserve each provider's reviewed URL:

The catalog contains 54 configured sources. Supported scope describes the
selected products and regions; it does not certify complete outage history or
every upstream lifecycle variant. Provider-wide events can remain in stored
history even when they fall outside the configured component scope.

Incident history and maintenance history can come from separate endpoints.
Some sources expose only a bounded recent archive, and missing recovery times
remain unknown. A successful poll or a zero affected-time total does not prove
uninterrupted uptime. See [provider adapter behavior](docs/backend.md#provider-catalog-and-adapter-contracts)
for collection limits. The Status.io adapter used by GitLab reads its current
feed without a separate archive importer.

| Provider | Supported scope | Official status source | Adapter |
| --- | --- | --- | --- |
| Jira | Jira Cloud (global feed) | [jira-software.status.atlassian.com](https://jira-software.status.atlassian.com) | Atlassian Statuspage |
| Jira Service Management | JSM Cloud (global feed) | [jira-service-management.status.atlassian.com](https://jira-service-management.status.atlassian.com) | Atlassian Statuspage |
| Airtable | Airtable service | [status.airtable.com](https://status.airtable.com) | Atlassian Statuspage |
| Asana | US services | [status.asana.com](https://status.asana.com) | Atlassian Statuspage |
| Ashby | Core products (global feed) | [status.ashbyhq.com](https://status.ashbyhq.com) | Atlassian Statuspage |
| Avalara | AvaTax, Returns, exemption management, connectors, account, e-invoicing, and developer services | [status.avalara.com](https://status.avalara.com) | Atlassian Statuspage |
| AWS | Core services in US East (N. Virginia and Ohio), US West (N. California and Oregon), and global services | [health.aws.amazon.com](https://health.aws.amazon.com) | AWS public current and historical health feeds |
| Bitbucket | Core Bitbucket Cloud services | [bitbucket.status.atlassian.com](https://bitbucket.status.atlassian.com) | Atlassian Statuspage |
| CircleCI | Jobs, pipelines, APIs, runners, and UI | [status.circleci.com](https://status.circleci.com) | Atlassian Statuspage |
| Claude | Claude products, API, Console, and Code | [status.claude.com](https://status.claude.com) | Atlassian Statuspage |
| Cloudflare | US network locations | [cloudflarestatus.com](https://www.cloudflarestatus.com) | Atlassian Statuspage |
| Confluence | Core Confluence Cloud services | [confluence.status.atlassian.com](https://confluence.status.atlassian.com) | Atlassian Statuspage |
| Console AI | Web App, Slackbot, and Assistant (shared feed; no regional breakdown) | [status.console.com](https://status.console.com) | Atlassian Statuspage |
| Datadog | Core US1 services | [status.datadoghq.com](https://status.datadoghq.com) | Atlassian Statuspage |
| DigitalOcean | US regions and global compute, storage, networking, database, and AI services | [status.digitalocean.com](https://status.digitalocean.com) | Atlassian Statuspage |
| Discord | Core, client, voice-region, and monetization services | [discordstatus.com](https://discordstatus.com) | Atlassian Statuspage |
| Docker | Docker Hub, platform, AI, and web services | [dockerstatus.com](https://www.dockerstatus.com) | Statuspage-compatible API |
| Dropbox | Applications, products, API, MCP, and support services | [status.dropbox.com](https://status.dropbox.com) | Atlassian Statuspage |
| Figma | Core Figma products and developer services | [status.figma.com](https://status.figma.com) | Atlassian Statuspage |
| Gainsight | Gainsight CS US1 and US2 applications, queues, integrations, and authentication | [status.gainsight.com](https://status.gainsight.com) | Atlassian Statuspage |
| Gemini | Gemini API, Code Assist, and Enterprise | [status.cloud.google.com](https://status.cloud.google.com) | Google Cloud Service Health JSON |
| GitHub | GitHub services | [githubstatus.com](https://www.githubstatus.com) | Atlassian Statuspage |
| GitLab | GitLab.com services | [status.gitlab.com](https://status.gitlab.com) | Status.io API |
| Gong | Core products and applications (global feed) | [status.gong.io](https://status.gong.io) | Incident.io Statuspage-compatible API |
| Google Cloud | Core compute, storage, networking, data, and operations services | [status.cloud.google.com](https://status.cloud.google.com) | Google Cloud Service Health JSON |
| Grafana Cloud | Core AWS US East and global services | [status.grafana.com](https://status.grafana.com) | Atlassian Statuspage |
| HubSpot | HubSpot products and APIs | [status.hubspot.com](https://status.hubspot.com) | Atlassian Statuspage |
| Intercom | US hosting: Fin AI Agent, inbox, messengers, developer and support services | [finstatus.com/us-hosting](https://www.finstatus.com/us-hosting) | Incident.io native US hosting API |
| LaunchDarkly | Flag delivery, feature management, and observability | [status.launchdarkly.com](https://status.launchdarkly.com) | Atlassian Statuspage |
| Mailgun | Email delivery, APIs, SMTP, validation, and analytics | [status.mailgun.com](https://status.mailgun.com) | Atlassian Statuspage |
| Adobe Marketo Engage | US data centers | [status.adobe.com](https://status.adobe.com) | Adobe status API |
| MongoDB Atlas | Core Atlas and VoyageAI services | [status.mongodb.com](https://status.mongodb.com) | Atlassian Statuspage |
| NetSuite | US data centers and services | [status.netsuite.com](https://status.netsuite.com) | Atlassian Statuspage |
| Notion | Core, developer, AI, and product services | [notion-status.com](https://www.notion-status.com) | Incident.io Statuspage-compatible API |
| Okta | US production and preview cells | [status.okta.com](https://status.okta.com) | Okta status page |
| 1Password | US and global product, identity, and developer services | [status.1password.com](https://status.1password.com) | Atlassian Statuspage |
| OpenAI | API, product, and Codex services | [status.openai.com](https://status.openai.com) | Statuspage-compatible API |
| PagerDuty | Core US, EU, and automation services | [status.pagerduty.com](https://status.pagerduty.com) | PagerDuty Status Pages API |
| Postman | Core platform, API, automation, and developer tools | [status.postman.com](https://status.postman.com) | Atlassian Statuspage |
| Rippling | Platform, identity, device, data, time, payroll, and benefits services | [status.rippling.com](https://status.rippling.com) | Atlassian Statuspage |
| Salesforce | Core platform, Data Cloud, Agentforce, Commerce, Marketing, Tableau, and MuleSoft | [status.salesforce.com](https://status.salesforce.com) | Salesforce Trust API |
| Sentry | US ingestion, alerting, monitoring, and global services | [status.sentry.io](https://status.sentry.io) | Atlassian Statuspage |
| Shopify | Admin, checkout, storefront, APIs, POS, and Oxygen | [shopifystatus.com](https://www.shopifystatus.com) | Atlassian Statuspage |
| Sigma | AWS, Azure, and GCP US deployments | [status.sigmacomputing.com](https://status.sigmacomputing.com) | Atlassian Statuspage |
| Slack | Slack services | [slack-status.com](https://slack-status.com) | Slack status API |
| Snowflake | AWS US East and Azure East US 2 services | [status.snowflake.com](https://status.snowflake.com) | Atlassian Statuspage |
| Stripe | Stripe APIs, payments, finance, and banking services | [status.stripe.com](https://status.stripe.com) | Atlassian Statuspage |
| Supabase | Core platform and US regions | [status.supabase.com](https://status.supabase.com) | Atlassian Statuspage |
| Twilio | Messaging, Voice US1, account security, email, and developer services | [status.twilio.com](https://status.twilio.com) | Atlassian Statuspage |
| Trello | Trello web application and API | [trello.status.atlassian.com](https://trello.status.atlassian.com) | Atlassian Statuspage |
| Vercel | Core platform and US CDN services | [vercel-status.com](https://www.vercel-status.com) | Atlassian Statuspage |
| Workato | Americas services | [status.workato.com](https://status.workato.com) | Atlassian Statuspage |
| Zapier | Automation, platform, AI, and developer services | [status.zapier.com](https://status.zapier.com) | Atlassian Statuspage |
| Zoom | Core Zoom products and REST API | [zoomstatus.com](https://www.zoomstatus.com) | Atlassian Statuspage |
