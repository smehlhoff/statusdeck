import { useResolvedTheme } from "../profile/theme";
import { useQuery } from "@tanstack/react-query";
import Chart from "chart.js/auto";
import { useEffect, useId, useRef, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { Analytics as AnalyticsData } from "../../api/types";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { MetricCard } from "../../components/MetricCard";
import { formatDuration, resolvedTimeZone } from "../../utils/display";
import { useProfile } from "../profile/profileContext";

const PERIODS = ["7d", "30d", "90d", "365d"] as const;
const SCOPES = ["monitored", "all"] as const;
const IMPACTED_COMPONENT_LIMIT = 10;

type Period = (typeof PERIODS)[number];
type Scope = (typeof SCOPES)[number];

const PROVIDER_COLUMNS = [
  "Provider",
  "Incidents",
  "Major",
  "Minor",
  "Active",
  "Resolved",
  "Affected time",
  "Median restore",
  "p95 restore",
  "p99 restore",
] as const;

type ProviderColumn = (typeof PROVIDER_COLUMNS)[number];
type ProviderRecord = AnalyticsData["providers"][number];
type ProviderSort = {
  column: ProviderColumn;
  direction: "ascending" | "descending";
};

function providerSortValue(
  provider: ProviderRecord,
  column: ProviderColumn,
): string | number | null {
  switch (column) {
    case "Provider":
      return provider.name;
    case "Incidents":
      return provider.incident_count;
    case "Major":
      return provider.major_incident_count;
    case "Minor":
      return Math.max(
        provider.incident_count - provider.major_incident_count,
        0,
      );
    case "Active":
      return Math.max(
        provider.incident_count - provider.resolved_incident_count,
        0,
      );
    case "Resolved":
      return provider.resolved_incident_count;
    case "Affected time":
      return provider.affected_seconds === 0 &&
        provider.unknown_duration_count > 0
        ? null
        : provider.affected_seconds;
    case "Median restore":
      return provider.median_restore_seconds;
    case "p95 restore":
      return provider.p95_restore_seconds;
    case "p99 restore":
      return provider.p99_restore_seconds;
  }
}

function compareProviders(
  left: ProviderRecord,
  right: ProviderRecord,
  sort: ProviderSort,
): number {
  const leftValue = providerSortValue(left, sort.column);
  const rightValue = providerSortValue(right, sort.column);
  if (leftValue === null && rightValue !== null) return 1;
  if (leftValue !== null && rightValue === null) return -1;
  let comparison = 0;
  if (typeof leftValue === "string" && typeof rightValue === "string") {
    comparison = leftValue.localeCompare(rightValue);
  } else if (typeof leftValue === "number" && typeof rightValue === "number") {
    comparison = leftValue - rightValue;
  }
  return (
    comparison * (sort.direction === "ascending" ? 1 : -1) ||
    left.name.localeCompare(right.name) ||
    left.provider_id.localeCompare(right.provider_id)
  );
}

function isPeriod(value: string | null): value is Period {
  return PERIODS.some((period) => period === value);
}

function isScope(value: string | null): value is Scope {
  return SCOPES.some((scope) => scope === value);
}

function incidentLink(
  data: AnalyticsData,
  selection: {
    providerId?: string;
    componentId?: string;
    from?: string;
    to?: string;
  } = {},
): string {
  const params = new URLSearchParams({
    from: selection.from ?? data.period.from,
    to: selection.to ?? data.period.to,
    scope: data.filters.scope,
  });
  if (!data.filters.include_maintenance) params.set("kind", "incident");
  if (selection.providerId) params.set("provider_id", selection.providerId);
  if (selection.componentId) params.set("component_id", selection.componentId);
  return `/incidents?${params.toString()}`;
}

export function Analytics() {
  const [providerSort, setProviderSort] = useState<ProviderSort>({
    column: "Affected time",
    direction: "descending",
  });
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedPeriod = searchParams.get("period");
  const requestedScope = searchParams.get("scope");
  const period = isPeriod(requestedPeriod) ? requestedPeriod : "30d";
  const scope = isScope(requestedScope) ? requestedScope : "monitored";
  const includeMaintenance = searchParams.get("maintenance") === "true";
  const requestParams = new URLSearchParams({ period, scope });
  if (includeMaintenance) requestParams.set("include_maintenance", "true");
  const filterQuery = requestParams.toString();
  const query = useQuery({
    queryKey: queryKeys.analytics(filterQuery),
    queryFn: ({ signal }) =>
      api<AnalyticsData>(`/api/v1/analytics?${filterQuery}`, { signal }),
  });

  function setFilter(key: "period" | "scope" | "maintenance", value: string) {
    const next = new URLSearchParams(searchParams);
    const isDefault =
      (key === "period" && value === "30d") ||
      (key === "scope" && value === "monitored") ||
      (key === "maintenance" && value === "false");
    if (isDefault) next.delete(key);
    else next.set(key, value);
    setSearchParams(next, { replace: true });
  }

  if (query.isLoading) {
    return <LoadingSkeleton label="Loading reliability analytics" rows={6} />;
  }
  if (!query.data) {
    return (
      <EmptyState
        title="Analytics unavailable"
        description="Reliability analytics could not be loaded. Check the connection and try again."
        error
        action={
          <button
            className="button ghost"
            type="button"
            onClick={() => void query.refetch()}
          >
            Retry
          </button>
        }
      />
    );
  }

  const sortIndicator = providerSort.direction === "ascending" ? " ↑" : " ↓";
  const data = query.data;
  const sortedProviders = [...data.providers].sort((left, right) =>
    compareProviders(left, right, providerSort),
  );
  if (data.providers.length === 0) {
    return (
      <>
        <AnalyticsHeading
          period={period}
          scope={scope}
          includeMaintenance={includeMaintenance}
          setFilter={setFilter}
          unknownDurationCount={data.data_quality.unknown_duration_count}
        />
        <EmptyState
          title="No monitored providers"
          description="Subscribe to providers before comparing their reliability."
          action={
            <Link className="button ghost" to="/catalog">
              Browse providers
            </Link>
          }
        />
      </>
    );
  }

  return (
    <>
      <AnalyticsHeading
        period={period}
        scope={scope}
        includeMaintenance={includeMaintenance}
        setFilter={setFilter}
        unknownDurationCount={data.data_quality.unknown_duration_count}
      />
      {query.isError && (
        <p className="alert error" role="alert">
          Refresh failed. Showing the last analytics snapshot.
        </p>
      )}
      <section
        className="analytics-summary-grid"
        aria-label="Reliability summary"
      >
        <MetricCard
          label="Reported affected time"
          value={
            data.summary.affected_seconds === 0 &&
            data.data_quality.unknown_duration_count > 0
              ? "Unknown"
              : formatDuration(data.summary.affected_seconds)
          }
          note="Comparisons unavailable: incomplete history"
        />
        <MetricCard
          label={
            includeMaintenance
              ? "Events (including maintenance)"
              : "Recorded incidents"
          }
          value={String(data.summary.incident_count)}
          note={`${data.summary.major_incident_count} major · ${data.summary.minor_incident_count} minor`}
        />
        <MetricCard
          label="Median restore"
          value={formatDuration(data.summary.median_restore_seconds)}
          note={
            data.summary.p95_restore_seconds === null
              ? "No reliable recovery durations"
              : `p95 ${formatDuration(data.summary.p95_restore_seconds)}`
          }
        />
      </section>

      <div className="analytics-main-grid">
        <TrendChart data={data} />
        <section className="card impacted-components-card">
          <div className="section-heading">
            <div>
              <h2>Top components with recorded incidents</h2>
            </div>
          </div>
          {data.impacted_components.length === 0 ? (
            <p className="analytics-empty muted">
              No component-level incidents are stored for this period.
            </p>
          ) : (
            <ul className="impacted-component-list">
              {[...data.impacted_components]
                .sort(
                  (left, right) =>
                    right.incident_count - left.incident_count ||
                    left.provider_name.localeCompare(right.provider_name) ||
                    left.name.localeCompare(right.name) ||
                    left.id.localeCompare(right.id),
                )
                .slice(0, IMPACTED_COMPONENT_LIMIT)
                .map((component) => (
                  <li key={component.id}>
                    <div>
                      <Link
                        to={incidentLink(data, {
                          providerId: component.provider_id,
                          componentId: component.id,
                        })}
                      >
                        {component.name}
                      </Link>
                      <small>{component.provider_name}</small>
                    </div>
                    <div className="impacted-component-metrics">
                      <strong>
                        {formatDuration(component.affected_seconds)}
                      </strong>
                      <small>
                        {component.incident_count} incident
                        {component.incident_count === 1 ? "" : "s"}
                      </small>
                    </div>
                  </li>
                ))}
            </ul>
          )}
        </section>
      </div>

      <section className="card provider-reliability-card">
        <div className="section-heading">
          <div>
            <h2>Provider records</h2>
            <p className="muted">
              Incident impact and restoration for every monitored provider.
            </p>
          </div>
        </div>
        <div
          className="table-wrap source-table-wrap"
          role="region"
          aria-label="Provider reliability metrics"
          tabIndex={0}
        >
          <table className="data-table responsive-data-table">
            <thead>
              <tr>
                {PROVIDER_COLUMNS.map((label) => (
                  <th
                    scope="col"
                    key={label}
                    aria-sort={
                      providerSort.column === label
                        ? providerSort.direction
                        : "none"
                    }
                  >
                    <button
                      type="button"
                      className="analytics-sort-button"
                      onClick={() =>
                        setProviderSort((current) => {
                          let direction: ProviderSort["direction"] =
                            label === "Provider" ? "ascending" : "descending";
                          if (current.column === label) {
                            direction =
                              current.direction === "ascending"
                                ? "descending"
                                : "ascending";
                          }
                          return { column: label, direction };
                        })
                      }
                    >
                      {label}
                      <span aria-hidden="true">
                        {providerSort.column === label ? sortIndicator : " ↕"}
                      </span>
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {sortedProviders.map((provider) => (
                <tr key={provider.provider_id}>
                  <td data-label="Provider">
                    <Link
                      className="event-id-button source-name-button"
                      to={incidentLink(data, {
                        providerId: provider.provider_id,
                      })}
                    >
                      {provider.name}
                    </Link>
                  </td>
                  <td data-label="Incidents">{provider.incident_count}</td>
                  <td data-label="Major">{provider.major_incident_count}</td>
                  <td data-label="Minor">
                    {Math.max(
                      provider.incident_count - provider.major_incident_count,
                      0,
                    )}
                  </td>
                  <td data-label="Active">
                    {Math.max(
                      provider.incident_count -
                        provider.resolved_incident_count,
                      0,
                    )}
                  </td>
                  <td data-label="Resolved">
                    {provider.resolved_incident_count}
                  </td>
                  <td data-label="Affected time">
                    {provider.affected_seconds === 0 &&
                    provider.unknown_duration_count > 0
                      ? "Unknown"
                      : formatDuration(provider.affected_seconds)}
                    {provider.unknown_duration_count > 0 && (
                      <small className="muted">
                        {" "}
                        · {provider.unknown_duration_count} unknown durations
                      </small>
                    )}
                  </td>
                  <td data-label="Median restore">
                    {formatDuration(provider.median_restore_seconds)}
                  </td>
                  <td data-label="p95 restore">
                    {formatDuration(provider.p95_restore_seconds)}
                  </td>
                  <td data-label="p99 restore">
                    {formatDuration(provider.p99_restore_seconds)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}

function AnalyticsHeading({
  period,
  scope,
  includeMaintenance,
  setFilter,
  unknownDurationCount,
}: {
  period: Period;
  scope: Scope;
  includeMaintenance: boolean;
  unknownDurationCount: number;
  setFilter: (key: "period" | "scope" | "maintenance", value: string) => void;
}) {
  const [showMethod, setShowMethod] = useState(false);
  const durationHelpId = useId();

  return (
    <>
      <div className="page-heading analytics-heading">
        <div>
          <div className="field-help-label">
            <h1>Analytics</h1>
            {unknownDurationCount > 0 && (
              <>
                <button
                  type="button"
                  className="field-help-button field-warning-button"
                  aria-label="Unknown incident durations"
                  aria-describedby={durationHelpId}
                >
                  <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    aria-hidden="true"
                  >
                    <path d="M12 3 2 21h20L12 3Z" />
                    <path d="M12 9v5m0 3h.01" />
                  </svg>
                </button>
                <span
                  className="field-help-tooltip"
                  id={durationHelpId}
                  role="tooltip"
                >
                  Duration unknown for {unknownDurationCount} recorded event
                  {unknownDurationCount === 1 ? "" : "s"}. These remain in
                  counts but are excluded from affected time and restore
                  statistics because their required timestamps are missing or
                  inconsistent.
                </span>
              </>
            )}
          </div>
          <p className="muted">
            Understand vendor-reported disruption across the providers you
            monitor.{" "}
            <button
              className="analytics-method-link"
              type="button"
              onClick={() => setShowMethod(true)}
            >
              How calculations work
            </button>
          </p>
        </div>
        <div className="analytics-heading-actions">
          <div className="analytics-controls" aria-label="Analytics filters">
            <label className="analytics-filter-control">
              <select
                aria-label="Reporting period"
                value={period}
                onChange={(event) => setFilter("period", event.target.value)}
              >
                <option value="7d">Last 7 days</option>
                <option value="30d">Last 30 days</option>
                <option value="90d">Last 90 days</option>
                <option value="365d">Last 365 days</option>
              </select>
            </label>
            <label className="analytics-filter-control">
              <select
                aria-label="Provider scope"
                value={scope}
                onChange={(event) => setFilter("scope", event.target.value)}
              >
                <option value="monitored">Monitored scope</option>
                <option value="all">All provider scope</option>
              </select>
            </label>
            <label className="analytics-filter-control analytics-toggle">
              <strong>Include maintenance</strong>
              <input
                type="checkbox"
                aria-label="Include maintenance"
                checked={includeMaintenance}
                onChange={(event) =>
                  setFilter("maintenance", String(event.target.checked))
                }
              />
            </label>
          </div>
        </div>
      </div>
      {showMethod && (
        <AnalyticsMethodDialog onClose={() => setShowMethod(false)} />
      )}
    </>
  );
}

function AnalyticsMethodDialog({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    dialog?.showModal();
    return () => {
      if (dialog?.open) dialog.close();
    };
  }, []);

  return (
    <dialog
      ref={dialogRef}
      className="analytics-method-dialog"
      aria-labelledby={titleId}
      onCancel={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="section-heading">
        <div>
          <p className="eyebrow">Metric methodology</p>
          <h2 id={titleId}>How calculations work</h2>
        </div>
        <button
          className="button ghost compact"
          type="button"
          onClick={onClose}
        >
          Close
        </button>
      </div>
      <p>
        <strong>Duration:</strong> Reported impact start to resolution, or now
        while ongoing. If the start is missing, incidents use first publication,
        never import time. This reported window may differ from the actual
        outage.
      </p>
      <p>
        <strong>Affected time:</strong> Counts only time within the selected
        period. Overlaps count once per provider or component; provider totals
        are added together. Major takes precedence over minor.
      </p>
      <p>
        <strong>Restore statistics:</strong> Median, p95 and p99 use full
        durations of resolved incidents overlapping the period, including
        first-report fallbacks.
      </p>
      <p>
        <strong>Unknown duration:</strong> Missing or inconsistent timestamps.
        Events remain in counts but add no affected time or restore statistics.
      </p>
      <p>
        <strong>Maintenance:</strong> Excluded unless enabled. Future scheduled
        maintenance shows “Not started.” Planned windows are not actual impact;
        duration requires an explicit start. Expired, unconfirmed windows remain
        uncertain.
      </p>
    </dialog>
  );
}

function TrendChart({ data }: { data: AnalyticsData }) {
  const { preferences } = useProfile();
  const theme = useResolvedTheme();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const navigate = useNavigate();
  const maximum = Math.max(
    ...data.trend.map((bucket) => bucket.major_seconds + bucket.minor_seconds),
    0,
  );

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || maximum === 0) return;
    const styles = getComputedStyle(document.documentElement);
    const dateFormatter = new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      timeZone: resolvedTimeZone(preferences),
    });

    const chart = new Chart(canvas, {
      type: "bar",
      data: {
        labels: data.trend.map((bucket) =>
          dateFormatter.format(new Date(bucket.from)),
        ),
        datasets: [
          {
            label: "Minor",
            data: data.trend.map((bucket) => bucket.minor_seconds),
            backgroundColor: styles.getPropertyValue("--chart-minor").trim(),
            borderRadius: 3,
            borderSkipped: false,
          },
          {
            label: "Major",
            data: data.trend.map((bucket) => bucket.major_seconds),
            backgroundColor: styles.getPropertyValue("--chart-major").trim(),
            borderRadius: 3,
            borderSkipped: false,
          },
        ],
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        interaction: {
          mode: "index",
          intersect: false,
        },
        animation: {
          duration: 250,
        },
        onClick: (_event, elements) => {
          const index = elements[0]?.index;
          const bucket = index === undefined ? undefined : data.trend[index];
          if (!bucket) return;
          navigate(
            incidentLink(data, {
              from: bucket.from,
              to: bucket.to,
            }),
          );
        },
        onHover: (event, elements) => {
          const target = event.native?.target;
          if (target instanceof HTMLElement) {
            target.style.cursor = elements.length > 0 ? "pointer" : "default";
          }
        },
        plugins: {
          legend: {
            display: false,
          },
          tooltip: {
            callbacks: {
              label: (context) =>
                `${context.dataset.label}: ${formatDuration(Number(context.raw))}`,
              footer: (items) => {
                const index = items[0]?.dataIndex;
                const bucket =
                  index === undefined ? undefined : data.trend[index];
                if (!bucket) return "";
                return `Total: ${formatDuration(bucket.major_seconds + bucket.minor_seconds)} · Select to view incidents`;
              },
            },
          },
        },
        scales: {
          x: {
            stacked: true,
            border: {
              display: false,
            },
            grid: {
              display: false,
            },
            ticks: {
              color: styles.getPropertyValue("--muted").trim(),
              maxRotation: 0,
              autoSkipPadding: 16,
              font: {
                size: 10,
              },
            },
          },
          y: {
            stacked: true,
            beginAtZero: true,
            border: {
              display: false,
            },
            grid: {
              color: styles.getPropertyValue("--chart-grid").trim(),
            },
            ticks: {
              color: styles.getPropertyValue("--muted").trim(),
              callback: (value) => formatDuration(Number(value)),
              font: {
                size: 10,
              },
            },
          },
        },
      },
    });

    return () => chart.destroy();
  }, [data, maximum, navigate, preferences, theme]);

  return (
    <section className="card analytics-chart-card">
      <div className="section-heading">
        <div>
          <h2>Affected time by {data.period.bucket}</h2>
          <p className="muted">
            Recorded impact with known duration
            {data.filters.include_maintenance
              ? " and planned maintenance"
              : ", excluding planned maintenance"}
            .
          </p>
        </div>
        <div className="analytics-legend" aria-label="Chart legend">
          <span className="major">Major</span>
          <span className="minor">Minor</span>
        </div>
      </div>
      {maximum === 0 ? (
        <div className="analytics-chart-empty">
          No known affected time is stored for this period. History or durations
          may be incomplete.
        </div>
      ) : (
        <div className="analytics-chart">
          <canvas
            ref={canvasRef}
            role="img"
            aria-label={`Stacked bar chart of major and minor affected time in hours by ${data.period.bucket}. Select a bar to view matching incidents.`}
          />
        </div>
      )}
    </section>
  );
}
