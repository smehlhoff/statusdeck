use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    shared::{authenticated_user, validate_optional_filter},
};

const DEFAULT_PERIOD: &str = "30d";
const DEFAULT_SCOPE: &str = "monitored";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AnalyticsQuery {
    period: Option<String>,
    scope: Option<String>,
    include_maintenance: Option<bool>,
}

#[derive(Debug, FromRow)]
struct MonitoredProviderRow {
    id: Uuid,
    name: String,
    history_available_from: Option<DateTime<Utc>>,
    history_refreshed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, FromRow)]
struct IncidentProviderRow {
    incident_id: Uuid,
    provider_id: Uuid,
    severity: String,
    resolved: bool,
    count_at: DateTime<Utc>,
    start_at: Option<DateTime<Utc>>,
    end_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct IncidentComponentRow {
    incident_id: Uuid,
    component_id: Uuid,
    provider_id: Uuid,
    provider_name: String,
    component_name: String,
}

#[derive(Clone, Debug)]
struct IncidentInterval {
    incident_id: Uuid,
    severity: SeverityBand,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    full_duration_seconds: Option<i64>,
    duration_unknown: bool,
    resolved: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SeverityBand {
    Minor,
    Major,
}

#[derive(Debug, Serialize)]
pub(super) struct AnalyticsResponse {
    period: AnalyticsPeriodResponse,
    filters: AnalyticsFiltersResponse,
    summary: AnalyticsSummaryResponse,
    trend: Vec<AnalyticsTrendResponse>,
    impacted_components: Vec<ImpactedComponentResponse>,
    providers: Vec<ProviderReliabilityResponse>,
    data_quality: AnalyticsDataQualityResponse,
}

#[derive(Debug, Serialize)]
struct AnalyticsPeriodResponse {
    key: String,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: &'static str,
}

#[derive(Debug, Serialize)]
struct AnalyticsFiltersResponse {
    scope: String,
    include_maintenance: bool,
}

#[derive(Debug, Serialize)]
struct AnalyticsSummaryResponse {
    affected_seconds: f64,
    incident_count: usize,
    major_incident_count: usize,
    minor_incident_count: usize,
    median_restore_seconds: Option<i64>,
    p90_restore_seconds: Option<i64>,
    p95_restore_seconds: Option<i64>,
    p99_restore_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
struct AnalyticsTrendResponse {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    major_seconds: f64,
    minor_seconds: f64,
}

#[derive(Debug, Serialize)]
struct ImpactedComponentResponse {
    id: Uuid,
    provider_id: Uuid,
    provider_name: String,
    name: String,
    affected_seconds: f64,
    incident_count: usize,
}

#[derive(Debug, Serialize)]
struct ProviderReliabilityResponse {
    provider_id: Uuid,
    name: String,
    incident_count: usize,
    major_incident_count: usize,
    resolved_incident_count: usize,
    affected_seconds: f64,
    median_restore_seconds: Option<i64>,
    p95_restore_seconds: Option<i64>,
    p99_restore_seconds: Option<i64>,
    unknown_duration_count: usize,
    history_available_from: Option<DateTime<Utc>>,
    history_refreshed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct AnalyticsDataQualityResponse {
    unknown_duration_count: usize,
    comparisons_available: bool,
}

pub(super) async fn analytics(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<AnalyticsQuery>,
) -> Result<Json<AnalyticsResponse>, ApiError> {
    let _ = authenticated_user(&state, &headers).await?;
    validate_optional_filter(
        "period",
        query.period.as_deref(),
        &["7d", "30d", "90d", "365d"],
    )?;
    validate_optional_filter("scope", query.scope.as_deref(), &["monitored", "all"])?;

    let period_key = query.period.as_deref().unwrap_or(DEFAULT_PERIOD);
    let scope = query.scope.as_deref().unwrap_or(DEFAULT_SCOPE);
    let include_maintenance = query.include_maintenance.unwrap_or(false);
    let period_days = period_days(period_key);
    let bucket_days = bucket_days(period_days);
    let to = Utc::now();
    let from = to - Duration::days(period_days);

    let providers = sqlx::query_as::<_, MonitoredProviderRow>(
        "SELECT provider.id, provider.name, source.history_available_from, source.history_refreshed_at
         FROM monitored_providers monitor
         JOIN providers provider ON provider.id = monitor.provider_id
         JOIN provider_sources source ON source.id = provider.provider_source_id
         WHERE monitor.enabled AND provider.active
         ORDER BY provider.name, provider.id",
    )
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;

    let incident_rows = sqlx::query_as::<_, IncidentProviderRow>(
        "SELECT incident.id AS incident_id,
                link.provider_id,
                incident.severity,
                incident.lifecycle = 'resolved' AS resolved,
                timing.count_at, timing.start_at, timing.end_at
         FROM incidents incident
         JOIN incident_timing timing ON timing.id = incident.id
         JOIN incident_providers link ON link.incident_id = incident.id
         JOIN providers provider ON provider.id = link.provider_id AND provider.active
         JOIN monitored_providers monitor ON monitor.provider_id = provider.id AND monitor.enabled
         WHERE ($4 OR incident.kind = 'incident')
           AND ($3 = 'all' OR (
               incident.within_provider_scope
               AND (
                   monitor.monitor_all_components
                   OR NOT EXISTS (
                       SELECT 1 FROM incident_components affected
                       WHERE affected.incident_id = incident.id
                   )
                   OR EXISTS (
                       SELECT 1
                       FROM incident_components affected
                       JOIN monitored_components selected
                         ON selected.component_id = affected.component_id
                        AND selected.monitored_provider_id = monitor.id
                       WHERE affected.incident_id = incident.id
                   )
               )
           ))
           AND timing.count_at < $1
           AND (timing.count_at >= $2 OR timing.end_at > $2)",
    )
    .bind(to)
    .bind(from)
    .bind(scope)
    .bind(include_maintenance)
    .fetch_all(&state.database.pool)
    .await
    .map_err(ApiError::internal)?;

    let current_incidents = unique_intervals(&incident_rows, from, to);
    let restoration_durations = restoration_durations(&current_incidents);
    let incident_ids = current_incidents
        .iter()
        .map(|interval| interval.incident_id)
        .collect::<Vec<_>>();

    let component_rows = if incident_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, IncidentComponentRow>(
            "SELECT DISTINCT affected.incident_id,
                    component.id AS component_id,
                    component.provider_id,
                    provider.name AS provider_name,
                    component.name AS component_name
             FROM incident_components affected
             JOIN components component ON component.id = affected.component_id
             JOIN providers provider ON provider.id = component.provider_id AND provider.active
             JOIN monitored_providers monitor ON monitor.provider_id = provider.id AND monitor.enabled
             WHERE affected.incident_id = ANY($1)
               AND ($2 = 'all' OR monitor.monitor_all_components OR EXISTS (
                   SELECT 1 FROM monitored_components selected
                   WHERE selected.monitored_provider_id = monitor.id
                     AND selected.component_id = component.id
               ))",
        )
        .bind(&incident_ids)
        .bind(scope)
        .fetch_all(&state.database.pool)
        .await
        .map_err(ApiError::internal)?
    };

    let provider_rows = provider_reliability(&providers, &incident_rows, from, to);
    let impacted_components = impacted_components(&component_rows, &current_incidents);
    let major_incident_count = current_incidents
        .iter()
        .filter(|interval| interval.severity == SeverityBand::Major)
        .count();
    let minor_incident_count = current_incidents.len() - major_incident_count;

    Ok(Json(AnalyticsResponse {
        period: AnalyticsPeriodResponse {
            key: period_key.to_owned(),
            from,
            to,
            bucket: match bucket_days {
                1 => "day",
                7 => "week",
                _ => "30-day period",
            },
        },
        filters: AnalyticsFiltersResponse {
            scope: scope.to_owned(),
            include_maintenance,
        },
        summary: AnalyticsSummaryResponse {
            affected_seconds: provider_rows
                .iter()
                .map(|provider| provider.affected_seconds)
                .sum(),
            incident_count: current_incidents.len(),
            major_incident_count,
            minor_incident_count,
            median_restore_seconds: percentile(&restoration_durations, 0.5),
            p90_restore_seconds: percentile(&restoration_durations, 0.9),
            p95_restore_seconds: percentile(&restoration_durations, 0.95),
            p99_restore_seconds: percentile(&restoration_durations, 0.99),
        },
        trend: trend(&incident_rows, from, to, bucket_days),
        impacted_components,
        providers: provider_rows,
        data_quality: AnalyticsDataQualityResponse {
            unknown_duration_count: current_incidents
                .iter()
                .filter(|interval| interval.duration_unknown)
                .count(),
            comparisons_available: false,
        },
    }))
}

fn period_days(period: &str) -> i64 {
    match period {
        "7d" => 7,
        "90d" => 90,
        "365d" => 365,
        _ => 30,
    }
}

const fn bucket_days(period_days: i64) -> i64 {
    match period_days {
        7 => 1,
        365 => 30,
        _ => 7,
    }
}

fn unique_intervals(
    rows: &[IncidentProviderRow],
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Vec<IncidentInterval> {
    let mut intervals = HashMap::new();
    for row in rows {
        if let Some(interval) = interval_for_row(row, range_start, range_end) {
            intervals.entry(row.incident_id).or_insert(interval);
        }
    }
    intervals.into_values().collect()
}

fn interval_for_row(
    row: &IncidentProviderRow,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Option<IncidentInterval> {
    let bounds = row
        .start_at
        .zip(row.end_at)
        .filter(|(start, end)| end >= start);
    let (start, end, full_duration_seconds, duration_unknown) = if let Some((start, end)) = bounds {
        if start >= range_end || end < range_start || (end == range_start && start < end) {
            return None;
        }
        (
            start.max(range_start),
            end.min(range_end),
            row.resolved.then(|| (end - start).num_seconds()),
            false,
        )
    } else {
        // Unknown duration contributes a count at the record's known date, never affected time.
        if row.count_at < range_start || row.count_at >= range_end {
            return None;
        }
        (row.count_at, row.count_at, None, true)
    };
    Some(IncidentInterval {
        incident_id: row.incident_id,
        severity: severity_band(&row.severity),
        start,
        end,
        full_duration_seconds,
        duration_unknown,
        resolved: row.resolved,
    })
}

fn severity_band(severity: &str) -> SeverityBand {
    match severity {
        "critical" | "major" => SeverityBand::Major,
        _ => SeverityBand::Minor,
    }
}

fn merged_duration_seconds(intervals: &[IncidentInterval]) -> f64 {
    let ranges = intervals
        .iter()
        .map(|interval| (interval.start, interval.end))
        .collect::<Vec<_>>();
    merged_range_seconds(ranges)
}

fn merged_range_seconds(mut ranges: Vec<(DateTime<Utc>, DateTime<Utc>)>) -> f64 {
    ranges.sort_unstable_by_key(|range| range.0);
    let Some((mut current_start, mut current_end)) = ranges.first().copied() else {
        return 0.0;
    };
    let mut total = Duration::zero();
    for (start, end) in ranges.into_iter().skip(1) {
        if start <= current_end {
            current_end = current_end.max(end);
        } else {
            total += current_end - current_start;
            current_start = start;
            current_end = end;
        }
    }
    (total + (current_end - current_start)).as_seconds_f64()
}

fn restoration_durations(intervals: &[IncidentInterval]) -> Vec<i64> {
    intervals
        .iter()
        .filter_map(|interval| interval.full_duration_seconds)
        .collect()
}

fn percentile(values: &[i64], percentile: f64) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values.get(index).copied()
}

fn trend(
    rows: &[IncidentProviderRow],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket_days: i64,
) -> Vec<AnalyticsTrendResponse> {
    let mut buckets = Vec::new();
    let mut bucket_start = from;
    while bucket_start < to {
        let bucket_end = (bucket_start + Duration::days(bucket_days)).min(to);
        let mut intervals_by_provider: HashMap<Uuid, Vec<IncidentInterval>> = HashMap::new();
        for row in rows {
            if let Some(interval) = interval_for_row(row, bucket_start, bucket_end) {
                intervals_by_provider
                    .entry(row.provider_id)
                    .or_default()
                    .push(interval);
            }
        }
        let (major_seconds, minor_seconds) = intervals_by_provider.values().fold(
            (0.0, 0.0),
            |(major_total, minor_total), intervals| {
                let (major, minor) = severity_durations(intervals, bucket_start, bucket_end);
                (major_total + major, minor_total + minor)
            },
        );
        buckets.push(AnalyticsTrendResponse {
            from: bucket_start,
            to: bucket_end,
            major_seconds,
            minor_seconds,
        });
        bucket_start = bucket_end;
    }
    buckets
}

fn severity_durations(
    intervals: &[IncidentInterval],
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> (f64, f64) {
    let ranges = intervals
        .iter()
        .filter_map(|interval| {
            let start = interval.start.max(range_start);
            let end = interval.end.min(range_end);
            (end > start).then_some((start, end, interval.severity))
        })
        .collect::<Vec<_>>();
    let major = merged_range_seconds(
        ranges
            .iter()
            .filter(|(_, _, severity)| *severity == SeverityBand::Major)
            .map(|(start, end, _)| (*start, *end))
            .collect(),
    );
    let total = merged_range_seconds(
        ranges
            .into_iter()
            .map(|(start, end, _)| (start, end))
            .collect(),
    );
    // Preserve fractional seconds through bucket aggregation. Major impact takes
    // precedence over minor; presentation may round after aggregation.
    (major, total - major)
}

fn provider_reliability(
    providers: &[MonitoredProviderRow],
    incident_rows: &[IncidentProviderRow],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<ProviderReliabilityResponse> {
    let mut intervals_by_provider: HashMap<Uuid, Vec<IncidentInterval>> = HashMap::new();
    for row in incident_rows {
        if let Some(interval) = interval_for_row(row, from, to) {
            intervals_by_provider
                .entry(row.provider_id)
                .or_default()
                .push(interval);
        }
    }
    let mut response = providers
        .iter()
        .map(|provider| {
            let intervals = intervals_by_provider
                .get(&provider.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let durations = restoration_durations(intervals);
            ProviderReliabilityResponse {
                provider_id: provider.id,
                name: provider.name.clone(),
                incident_count: intervals.len(),
                major_incident_count: intervals
                    .iter()
                    .filter(|interval| interval.severity == SeverityBand::Major)
                    .count(),
                resolved_incident_count: intervals
                    .iter()
                    .filter(|interval| interval.resolved)
                    .count(),
                unknown_duration_count: intervals
                    .iter()
                    .filter(|interval| interval.duration_unknown)
                    .count(),
                history_available_from: provider.history_available_from,
                history_refreshed_at: provider.history_refreshed_at,
                affected_seconds: merged_duration_seconds(intervals),
                median_restore_seconds: percentile(&durations, 0.5),
                p95_restore_seconds: percentile(&durations, 0.95),
                p99_restore_seconds: percentile(&durations, 0.99),
            }
        })
        .collect::<Vec<_>>();
    response.sort_by(|left, right| left.name.cmp(&right.name));
    response
}

fn impacted_components(
    component_rows: &[IncidentComponentRow],
    intervals: &[IncidentInterval],
) -> Vec<ImpactedComponentResponse> {
    let intervals_by_incident = intervals
        .iter()
        .map(|interval| (interval.incident_id, interval.clone()))
        .collect::<HashMap<_, _>>();
    let mut component_details = HashMap::new();
    let mut intervals_by_component: HashMap<Uuid, Vec<IncidentInterval>> = HashMap::new();
    let mut incident_ids_by_component: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for row in component_rows {
        let Some(interval) = intervals_by_incident.get(&row.incident_id) else {
            continue;
        };
        component_details
            .entry(row.component_id)
            .or_insert_with(|| {
                (
                    row.provider_id,
                    row.provider_name.clone(),
                    row.component_name.clone(),
                )
            });
        intervals_by_component
            .entry(row.component_id)
            .or_default()
            .push(interval.clone());
        incident_ids_by_component
            .entry(row.component_id)
            .or_default()
            .insert(row.incident_id);
    }
    let mut response = component_details
        .into_iter()
        .map(
            |(id, (provider_id, provider_name, name))| ImpactedComponentResponse {
                id,
                provider_id,
                provider_name,
                name,
                affected_seconds: intervals_by_component
                    .get(&id)
                    .map_or(0.0, |intervals| merged_duration_seconds(intervals)),
                incident_count: incident_ids_by_component.get(&id).map_or(0, HashSet::len),
            },
        )
        .collect::<Vec<_>>();
    response.sort_by(|left, right| {
        left.provider_name
            .cmp(&right.provider_name)
            .then_with(|| left.name.cmp(&right.name))
    });
    response
}
