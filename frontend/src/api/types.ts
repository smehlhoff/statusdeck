export type Status =
  | "operational"
  | "maintenance"
  | "degraded"
  | "partial_outage"
  | "major_outage"
  | "unknown";

export type Freshness =
  "fresh" | "delayed" | "stale" | "never_checked" | "failing";

export type Severity = "info" | "minor" | "major" | "critical";
export type IncidentLifecycle = "open" | "resolution_pending" | "resolved";
export type IncidentKind = "incident" | "maintenance";
export type ChannelType =
  "webhook" | "discord" | "slack" | "mattermost" | "gotify" | "ntfy" | "zulip";
export type RuleKind = "provider" | "system_health";
export type DeliveryStatus =
  | "pending"
  | "retrying"
  | "delivered"
  | "ambiguous"
  | "failed"
  | "held"
  | "summarized"
  | "cancelled"
  | "suppressed";
export type DashboardCountKey = Status | "stale";

export interface User {
  id: string;
  email: string;
  role: string;
  display_name: string;
  preferences: DisplayPreferences;
}

export interface DisplayPreferences {
  theme: "light" | "dark" | "system";
  time_zone: string;
  date_format: "locale" | "iso" | "day_first" | "month_first";
  time_format: "locale" | "twelve_hour" | "twenty_four_hour";
  timestamp_format: "relative" | "absolute";
  landing_page:
    | "/"
    | "/catalog"
    | "/incidents"
    | "/analytics"
    | "/notifications"
    | "/system"
    | "/profile";
  refresh_interval_ms: 0 | 5000 | 15000 | 60000;
}

export interface ProfileSession {
  id: string;
  created_at: string;
  last_seen_at: string;
  expires_at: string;
  user_agent: string | null;
  ip_address: string | null;
  status: "active" | "expired" | "ended";
  ended_at: string | null;
  current: boolean;
}

export interface SecurityActivity {
  id: string;
  action: string;
  created_at: string;
}

export interface CatalogProvider {
  id: string;
  slug: string;
  name: string;
  description: string;
  tags: string[];
  official_url: string;
  active: boolean;
}

export interface Component {
  id: string;
  upstream_id: string;
  name: string;
  description: string | null;
  group: string | null;
  position: number;
  active: boolean;
  status: Status | null;
  selected: boolean;
}

export interface RuleComponent {
  id: string;
  provider_id: string;
  name: string;
  group: string | null;
  monitored: boolean;
}

export interface CatalogDetail extends CatalogProvider {
  source_adapter: string;
  current_status: Status | null;
  provider_status: Status | null;
  freshness: Freshness;
  last_attempt_at: string | null;
  last_success_at: string | null;
  recent_poll_error: string | null;
  monitor: Monitor | null;
  components: Component[];
  active_incidents: Incident[];
}

export interface ReliabilityDay {
  started_major_count: number;
  started_minor_count: number;
  date: string;
  from: string;
  to: string;
  status: "healthy" | "minor" | "major" | "maintenance" | "unknown";
  incident_count: number;
  maintenance_count: number;
  affected_seconds: number;
  unknown_duration_count: number;
  unknown_incident_duration_count: number;
}

export interface ProviderReliability {
  history_available_from: string | null;
  history_refreshed_at: string | null;
  coverage_complete: boolean;
  from: string;
  to: string;
  days: ReliabilityDay[];
}

export interface Monitor {
  id: string;
  provider_id: string;
  monitor_all_components: boolean;
  enabled: boolean;
}

export interface DashboardProvider {
  id: string;
  slug: string;
  name: string;
  official_url: string;
  tags: string[];
  status: Status;
  provider_status: Status;
  freshness: Freshness;
  last_success_at: string | null;
  active_incidents: number;
  open_maintenance: number;
  upcoming_maintenance: number;
  outside_scope_incidents: number;
  affected_components: string[];
}

export interface Dashboard {
  counts: Record<DashboardCountKey, number>;
  providers: DashboardProvider[];
}

export interface Analytics {
  period: {
    key: "7d" | "30d" | "90d" | "365d";
    from: string;
    to: string;
    bucket: "day" | "week" | "30-day period";
  };
  filters: {
    scope: "monitored" | "all";
    include_maintenance: boolean;
  };
  summary: {
    affected_seconds: number;
    incident_count: number;
    major_incident_count: number;
    minor_incident_count: number;
    median_restore_seconds: number | null;
    p90_restore_seconds: number | null;
    p95_restore_seconds: number | null;
    p99_restore_seconds: number | null;
  };
  trend: Array<{
    from: string;
    to: string;
    major_seconds: number;
    minor_seconds: number;
  }>;
  impacted_components: Array<{
    id: string;
    provider_id: string;
    provider_name: string;
    name: string;
    affected_seconds: number;
    incident_count: number;
    unknown_duration_count: number;
  }>;
  providers: Array<{
    provider_id: string;
    name: string;
    incident_count: number;
    major_incident_count: number;
    resolved_incident_count: number;
    unknown_duration_count: number;
    history_available_from: string | null;
    history_refreshed_at: string | null;
    affected_seconds: number;
    median_restore_seconds: number | null;
    p95_restore_seconds: number | null;
    p99_restore_seconds: number | null;
  }>;
  data_quality: {
    unknown_duration_count: number;
    comparisons_available: boolean;
  };
}

export interface Incident {
  id: string;
  bookmarked: boolean;
  upstream_incident_id: string;
  title: string;
  lifecycle: IncidentLifecycle;
  severity: Severity;
  kind: IncidentKind;
  original_phase: string;
  original_impact: string | null;
  provider_created_at: string | null;
  provider_started_at: string | null;
  provider_updated_at: string | null;
  provider_monitoring_at: string | null;
  provider_resolved_at: string | null;
  provider_metadata: Record<string, unknown>;
  duration_seconds: number | null;
  maintenance_uncertain: boolean;
  planned_start_at: string | null;
  planned_end_at: string | null;
  first_observed_at: string;
  official_url: string | null;
  last_observed_at: string;
  lifecycle_generation: number;
  within_provider_scope: boolean;
  providers: Array<{ id: string; name: string }>;
}

export interface IncidentPage {
  items: Incident[];
  next_cursor: string | null;
}

export interface Bookmark {
  id: string;
  title: string;
  kind: IncidentKind;
  severity: Severity;
  lifecycle: IncidentLifecycle;
  providers: Array<{ id: string; name: string }>;
  bookmarked_at: string;
}

export interface BookmarkPage {
  items: Bookmark[];
  next_cursor: string | null;
  providers: Array<{ id: string; name: string }>;
}

export interface IncidentUpdate {
  id: string;
  status: string;
  body: string;
  created_at: string | null;
  updated_at: string | null;
  display_at: string | null;
  synthesized: boolean;
  components: Array<{ id: string; name: string }>;
  scopes: AffectedScope[];
}

export interface AffectedScope {
  type: string;
  upstream_id: string;
  name: string;
  normalized_status: Status | null;
  original_status: string | null;
}

export interface IncidentDetail {
  incident: Incident;
  providers: Array<{
    id: string;
    slug: string;
    name: string;
    official_url: string;
  }>;
  components: Array<{
    id: string;
    provider_id: string;
    name: string;
    normalized_status: Status | null;
    original_status: string | null;
  }>;
  scopes: AffectedScope[];
  updates: IncidentUpdate[];
}

export interface Source {
  id: string;
  source_key: string;
  adapter: string;
  enabled: boolean;
  next_poll_at: string;
  last_attempt_at: string | null;
  last_success_at: string | null;
  consecutive_failures: number;
  status_checks_24h: number;
  status_failures_24h: number;
  last_error: string | null;
  stale_episode_started_at: string | null;
  affected_providers: Array<{ id: string; slug: string; name: string }>;
  last_poll_outcome: string | null;
  last_poll_duration_ms: number | null;
  last_poll_http_status: number | null;
  last_poll_adapter_version: string | null;
  last_poll_error_class: string | null;
  freshness: Freshness;
}

export interface RecordCount {
  record_type: string;
  count: number;
}

export type SystemWindow = "1h" | "24h" | "7d";

export interface SystemSummary {
  observed_at: string;
  window: { hours: number; since: string; until: string };
  application_version: string;
  api_uptime_seconds: number;
  database: string;
  database_query_ms: number;
  database_metrics: {
    size: string;
    connections: number;
    active_queries: number | null;
    lock_waits: number | null;
    idle_transactions: number | null;
    longest_transaction_seconds: number | null;
  };
  database_connections: {
    open: number;
    idle: number;
    in_use: number;
    limit: number;
  };
  migration_version: number;
  migrations_current: boolean;
  heartbeat_tolerance_seconds: number;
  delivery_grace_seconds: number;
  worker_heartbeats: Array<{
    role: string;
    instance_id: string;
    version: string;
    heartbeat_at: string;
    started_at: string | null;
    last_completed_at: string | null;
    fresh: boolean;
    in_progress: number;
    expired_claims: number;
    oldest_started_at: string | null;
  }>;
  polling: {
    overdue: number;
    overdue_source_ids: string[];
    longest_delay_seconds: number | null;
    completed: number;
    successful: number;
    failed: number;
    median_ms: number | null;
    p95_ms: number | null;
    last_completed_at: string | null;
  };
  deliveries: {
    enabled_channels: number;
    last_attempt_at: string | null;
    current: {
      queued: number;
      paused: number;
      held: number;
      due_now: number;
      overdue: number;
      oldest_overdue_seconds: number | null;
      next_retry_at: string | null;
      retrying: number;
      ambiguous: number;
    };
    recent: {
      attempts: number;
      failed: number;
      ambiguous: number;
      delivered: number;
      latency_samples: number;
      delivered_after_hold: number;
      median_seconds: number | null;
      p95_seconds: number | null;
    };
    history: Partial<Record<DeliveryStatus, number>>;
  };
}

export interface Channel {
  id: string;
  name: string;
  channel_type: ChannelType;
  display_target: string;
  enabled: boolean;
  last_tested_at: string | null;
}

export interface QuietHours {
  enabled: boolean;
  timezone: string;
  days: number[];
  start: string;
  end: string;
  critical_override: boolean;
}

export interface Rule {
  quiet_hours: QuietHours;
  quiet_until: string | null;
  id: string;
  name: string;
  rule_kind: RuleKind;
  min_severity: Severity;
  notify_detected: boolean;
  notify_started: boolean;
  notify_updated: boolean;
  notify_resolved: boolean;
  notify_reopened: boolean;
  notify_maintenance: boolean;
  notify_recovered: boolean;
  enabled: boolean;
  channel_ids: string[];
  provider_ids: string[];
  component_ids: string[];
}

export interface Delivery {
  id: string | null;
  can_resend: boolean;
  resend_requested_at: string | null;
  notification_event_id: string;
  event_type: string;
  incident_id: string | null;
  payload: unknown;
  alert_rule_id: string | null;
  channel_id: string | null;
  channel_type: ChannelType | null;
  channel_name: string | null;
  channel_target: string | null;
  status: DeliveryStatus;
  suppression_reason: string | null;
  created_at: string;
  attempt_count: number;
  next_attempt_at: string | null;
  quiet_until: string | null;
  summary_delivery_id: string | null;
  last_error: string | null;
  delivered_at: string | null;
  last_attempt_at: string | null;
  last_response_class: string | null;
  last_response_status: number | null;
  last_attempt_ambiguous: boolean | null;
}

export interface IncidentComment {
  id: string;
  author_user_id: string;
  author_email: string;
  author_display_name: string;
  body: string;
  created_at: string;
  edited_at: string | null;
}

export interface IncidentCommentPage {
  items: IncidentComment[];
  next_cursor: string | null;
  total_count: number;
}

export interface DeliveryPage {
  items: Delivery[];
  next_cursor: string | null;
}
export interface NotificationSummary {
  event_id: string;
  created_at: string;
  payload: {
    title: string;
    quiet_until: string;
    event_count: number;
    omitted_entries: number;
    entries: {
      entity_id: string;
      event_type: string;
      title: string;
      status: string;
      providers: { id: string; name: string }[];
      started_at: string | null;
      resolved_at: string | null;
      last_event_at: string;
    }[];
  };
}

export interface MyComment {
  id: string;
  incident_id: string;
  title: string;
  body: string;
  kind: "incident" | "maintenance";
  severity: string;
  lifecycle: string;
  providers: { id: string; name: string }[];
  created_at: string;
  edited_at: string | null;
}

export interface MyCommentPage {
  items: MyComment[];
  next_cursor: string | null;
  providers: { id: string; name: string }[];
}

export interface DeliveryAttempt {
  id: string;
  attempted_at: string;
  outcome: string;
  response_status: number | null;
  error_message: string | null;
}
