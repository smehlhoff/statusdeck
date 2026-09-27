import { Fragment, type ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import type {
  Monitor,
  Source,
  SystemSummary,
  SystemWindow,
} from "../../api/types";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { formatDuration, humanizeIdentifier } from "../../utils/display";
import {
  healthClass,
  snapshotState,
  type HealthState,
} from "./diagnosticState";

interface Props {
  summary: UseQueryResult<SystemSummary>;
  sources: UseQueryResult<Source[]>;
  monitors: UseQueryResult<Monitor[]>;
  now: number;
  window: SystemWindow;
  onWindowChange: (window: SystemWindow) => void;
}

function rate(successful: number, total: number): string {
  return total
    ? `${((100 * successful) / total).toFixed(1)}% (${successful} / ${total})`
    : "No activity";
}

function perMinute(count: number, hours: number): string {
  const average = count / (hours * 60);
  return average > 0 && average < 0.01
    ? "<0.01"
    : average.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

function duration(value: number | null, divisor = 1): string {
  return value === null ? "No samples" : formatDuration(value / divisor);
}

function age(value: string | null, reference: string): ReactNode {
  return value ? (
    <span title={value}>
      {formatDuration(
        Math.max(0, (Date.parse(reference) - Date.parse(value)) / 1000),
      )}{" "}
      ago
    </span>
  ) : (
    "Not reported"
  );
}

export function RuntimeOverview(props: Props) {
  const { summary, sources, monitors, now, window: period } = props;
  const data = summary.data;
  const summaryState = snapshotState(summary, now);
  const sourceState = snapshotState(sources, now);
  const monitorState = snapshotState(monitors, now);
  const coverageState = sourceState !== "Healthy" ? sourceState : monitorState;
  const ids = new Set(
    monitors.data?.filter((m) => m.enabled).map((m) => m.provider_id),
  );
  const active =
    sources.data?.filter(
      (s) => s.enabled && s.affected_providers.some((p) => ids.has(p.id)),
    ) ?? [];
  const counts = {
    fresh: 0,
    delayed: 0,
    stale: 0,
    never_checked: 0,
    failing: 0,
  };
  for (const source of active) counts[source.freshness]++;
  const workerState = (role: string): HealthState => {
    if (summaryState !== "Healthy" || !data) return summaryState;
    const heartbeat = data.worker_heartbeats.find((h) => h.role === role);
    if (!heartbeat || !heartbeat.fresh) return "Unavailable";
    if (heartbeat.version !== data.application_version) return "Degraded";
    if (!heartbeat.last_completed_at) return "Unknown";
    return Date.parse(data.observed_at) -
      Date.parse(heartbeat.last_completed_at) >
      data.heartbeat_tolerance_seconds * 1000
      ? "Degraded"
      : "Healthy";
  };
  const pollerState = workerState("poller");
  const dispatcherState = workerState("dispatcher");
  const workersState =
    [pollerState, dispatcherState].find((s) => s === "Unavailable") ??
    [pollerState, dispatcherState].find((s) => s !== "Healthy") ??
    "Healthy";
  let feedState: HealthState = coverageState;
  if (coverageState === "Healthy") {
    if (ids.size === 0) feedState = "Not configured";
    else if (active.length === 0 || counts.fresh !== active.length)
      feedState = "Degraded";
    else if (summaryState !== "Healthy") feedState = summaryState;
    else if (data && data.polling.overdue > 0) feedState = "Degraded";
  }
  let databaseState: HealthState = summaryState;
  if (summaryState === "Healthy" && !data?.migrations_current)
    databaseState = "Degraded";
  let notificationState: HealthState = summaryState;
  if (summaryState === "Healthy" && data) {
    if (data.deliveries.enabled_channels === 0)
      notificationState = "Not configured";
    else if (dispatcherState !== "Healthy") notificationState = dispatcherState;
    else if (
      data.deliveries.current.overdue +
        data.deliveries.recent.failed +
        data.deliveries.recent.ambiguous >
      0
    )
      notificationState = "Degraded";
  }
  const sourceDetail =
    coverageState === "Healthy"
      ? `${counts.fresh} / ${active.length} fresh`
      : "Coverage could not be verified";
  const stages: Array<{
    label: string;
    status: HealthState;
    detail: string;
  }> = [
    {
      label: "API",
      status: summaryState,
      detail:
        summaryState === "Healthy"
          ? "Diagnostics responding"
          : "Waiting for diagnostics",
    },
    {
      label: "Workers",
      status: workersState,
      detail: `${data?.worker_heartbeats.filter((h) => h.fresh).length ?? 0} / 2 roles reporting`,
    },
    {
      label: "Provider feeds",
      status: feedState,
      detail: sourceDetail,
    },
    {
      label: "PostgreSQL",
      status: databaseState,
      detail: data
        ? `${data.database_metrics.connections} client connections`
        : "Waiting for diagnostics",
    },
    {
      label: "Notifications",
      status: notificationState,
      detail: data
        ? `${data.deliveries.current.overdue} overdue · ${data.deliveries.recent.failed} recent failures`
        : "Waiting for diagnostics",
    },
  ];
  return (
    <Fragment key={data ? period : "loading"}>
      <section
        className="card pipeline-card"
        aria-labelledby="system-health-heading"
      >
        <div className="section-heading">
          <div>
            <h2 id="system-health-heading">System health</h2>
          </div>
          <select
            className="button ghost compact"
            aria-label="Recent statistics"
            value={period}
            onChange={(e) =>
              props.onWindowChange(e.target.value as SystemWindow)
            }
          >
            <option value="1h">Last hour</option>
            <option value="24h">Last 24 hours</option>
            <option value="7d">Last 7 days</option>
          </select>
        </div>
        {summaryState !== "Healthy" && (
          <p className="alert" role="status">
            {data
              ? "This snapshot is out of date. Current health could not be verified."
              : "Runtime health has not been verified."}{" "}
            <button
              className="button ghost compact"
              disabled={summary.isFetching}
              onClick={() => void summary.refetch()}
            >
              Refresh
            </button>
          </p>
        )}
        <div className="system-pipeline">
          {stages.map((stage) => (
            <div
              key={stage.label}
              className={`card pipeline-stage ${healthClass(stage.status)}`}
            >
              <span>{stage.label}</span>
              <strong>{stage.status}</strong>
              <small>
                {summaryState !== "Healthy" && stage.label !== "Provider feeds"
                  ? "Current state unverified"
                  : stage.detail}
              </small>
            </div>
          ))}
        </div>
      </section>
      <section
        className="card system-detail-section"
        aria-labelledby="runtime-heading"
      >
        <div className="section-heading">
          <h2 id="runtime-heading">Runtime details</h2>
          <span className="muted">Backlog now · Recent activity: {period}</span>
        </div>
        <div className="system-detail-grid">
          <DetailCard
            id="runtime-coverage"
            title="Monitoring coverage"
            status={feedState}
          >
            <Detail
              label="Monitored providers"
              value={coverageState === "Healthy" ? ids.size : "Unknown"}
            />
            <Detail
              label="Active sources"
              value={coverageState === "Healthy" ? active.length : "Unknown"}
            />
            {coverageState === "Healthy" &&
              Object.entries(counts).map(([key, count]) => (
                <Detail
                  key={key}
                  label={
                    key === "never_checked"
                      ? "Never checked"
                      : key.charAt(0).toUpperCase() +
                        humanizeIdentifier(key).slice(1)
                  }
                  value={count}
                />
              ))}
          </DetailCard>
          <DetailCard
            id="runtime-database"
            title="PostgreSQL"
            status={databaseState}
          >
            <Detail
              label="Database size"
              value={data?.database_metrics.size ?? "Unknown"}
            />
            <Detail
              label="Client connections"
              value={data?.database_metrics.connections ?? "Unknown"}
            />
            <Detail
              label="Active client queries"
              value={data?.database_metrics.active_queries ?? "Unknown"}
            />
            <Detail
              label="Lock-waiting clients"
              value={data?.database_metrics.lock_waits ?? "Unknown"}
            />
            <Detail
              label="Idle transactions"
              value={data?.database_metrics.idle_transactions ?? "Unknown"}
            />
            <Detail
              label="Longest client transaction"
              value={
                data &&
                data.database_metrics.longest_transaction_seconds !== null
                  ? formatDuration(
                      data.database_metrics.longest_transaction_seconds,
                    )
                  : "Unknown"
              }
            />
            <Detail
              label="Query + pool wait"
              value={
                data ? formatDuration(data.database_query_ms / 1000) : "Unknown"
              }
            />
          </DetailCard>
          {(["poller", "dispatcher"] as const).map((role) => {
            const heartbeat = data?.worker_heartbeats.find(
              (h) => h.role === role,
            );
            let reporting = data ? "Not reporting" : "Unknown";
            if (heartbeat)
              reporting = heartbeat.fresh
                ? "Within tolerance"
                : "Heartbeat overdue";
            const activityVerified =
              summaryState === "Healthy" && heartbeat?.fresh;
            let activity = "Unverified";
            if (activityVerified && heartbeat) {
              activity =
                role === "poller"
                  ? "No checks in progress"
                  : "No deliveries in progress";
              if (heartbeat.in_progress > 0) {
                activity =
                  role === "poller"
                    ? `Checking ${heartbeat.in_progress} source${heartbeat.in_progress === 1 ? "" : "s"}`
                    : `Processing ${heartbeat.in_progress} deliver${heartbeat.in_progress === 1 ? "y" : "ies"}`;
              }
              if (heartbeat.expired_claims > 0)
                activity = "Unverified (expired work claim)";
            }
            let activeElapsed = "Unknown";
            if (
              activityVerified &&
              heartbeat &&
              heartbeat.expired_claims === 0 &&
              data
            ) {
              activeElapsed = heartbeat.in_progress === 0 ? "—" : "Unknown";
              if (heartbeat.oldest_started_at) {
                activeElapsed = formatDuration(
                  Math.max(
                    0,
                    (Date.parse(data.observed_at) -
                      Date.parse(heartbeat.oldest_started_at)) /
                      1000,
                  ),
                );
              }
            }
            return (
              <DetailCard
                key={role}
                id={`runtime-${role}`}
                title={role === "poller" ? "Poller" : "Dispatcher"}
                status={workerState(role)}
              >
                <Detail label="Reporting" value={reporting} />
                <Detail label="Current activity" value={activity} />
                <Detail
                  label="Oldest active work elapsed"
                  value={activeElapsed}
                />
                <Detail
                  label={`${role === "poller" ? "Checks" : "Delivery attempts"} / min (${period} avg)`}
                  value={
                    data
                      ? perMinute(
                          role === "poller"
                            ? data.polling.completed
                            : data.deliveries.recent.attempts,
                          data.window.hours,
                        )
                      : "Unknown"
                  }
                />
                <Detail
                  label="Heartbeat age / tolerance"
                  value={
                    heartbeat && data ? (
                      <>
                        {age(heartbeat.heartbeat_at, data.observed_at)} /{" "}
                        {formatDuration(data.heartbeat_tolerance_seconds)}
                      </>
                    ) : (
                      "Not reported"
                    )
                  }
                />
                <Detail
                  label="Last completed cycle"
                  value={
                    heartbeat && data
                      ? age(heartbeat.last_completed_at, data.observed_at)
                      : "Not reported"
                  }
                />
                <Detail
                  label="Uptime at snapshot"
                  value={
                    heartbeat?.started_at && data
                      ? formatDuration(
                          Math.max(
                            0,
                            (Date.parse(data.observed_at) -
                              Date.parse(heartbeat.started_at)) /
                              1000,
                          ),
                        )
                      : "Unknown"
                  }
                />
                {data &&
                  (role === "poller" ? (
                    <>
                      <Detail
                        label="Overdue sources"
                        value={data.polling.overdue}
                      />
                      <Detail
                        label="Longest scheduling delay"
                        value={formatDuration(
                          data.polling.longest_delay_seconds ?? 0,
                        )}
                      />
                      <Detail
                        label="Last completed poll"
                        value={age(
                          data.polling.last_completed_at,
                          data.observed_at,
                        )}
                      />
                      <Detail
                        label={`Poll success (${period})`}
                        value={rate(
                          data.polling.successful,
                          data.polling.completed,
                        )}
                      />
                      <Detail
                        label={`Failed polls (${period})`}
                        value={data.polling.failed}
                      />
                      <Detail
                        label="Poll duration median / p95"
                        value={`${duration(data.polling.median_ms, 1000)} / ${duration(data.polling.p95_ms, 1000)}`}
                      />
                    </>
                  ) : (
                    <>
                      <Detail
                        label="Deliveries due now"
                        value={data.deliveries.current.due_now}
                      />
                      <Detail
                        label="Last send attempt"
                        value={age(
                          data.deliveries.last_attempt_at,
                          data.observed_at,
                        )}
                      />
                      <Detail
                        label="Next scheduled retry"
                        value={
                          data.deliveries.current.next_retry_at ? (
                            <RelativeDateTime
                              value={data.deliveries.current.next_retry_at}
                            />
                          ) : (
                            "None scheduled"
                          )
                        }
                      />
                    </>
                  ))}
              </DetailCard>
            );
          })}
          <DetailCard
            id="runtime-notifications"
            title="Notifications"
            status={notificationState}
          >
            {data ? (
              <>
                <Detail
                  label="Enabled channels"
                  value={data.deliveries.enabled_channels}
                />
                <Detail
                  label="Queued (excluding held)"
                  value={data.deliveries.current.queued}
                />
                <Detail
                  label="Overdue deliveries"
                  value={data.deliveries.current.overdue}
                />
                <Detail
                  label="Oldest overdue"
                  value={formatDuration(
                    data.deliveries.current.oldest_overdue_seconds ?? 0,
                  )}
                />
                {data.deliveries.current.held > 0 && (
                  <Detail
                    label="Held for quiet hours"
                    value={data.deliveries.current.held}
                  />
                )}
                {data.deliveries.current.paused > 0 && (
                  <Detail
                    label="Paused by channel or rule"
                    value={data.deliveries.current.paused}
                  />
                )}
                {data.deliveries.current.retrying > 0 && (
                  <Detail
                    label="Retrying now"
                    value={data.deliveries.current.retrying}
                  />
                )}
                {data.deliveries.current.ambiguous > 0 && (
                  <Detail
                    label="Uncertain now"
                    value={data.deliveries.current.ambiguous}
                  />
                )}
                <Detail
                  label={`Delivery success (${period})`}
                  value={rate(
                    data.deliveries.recent.delivered,
                    data.deliveries.recent.delivered +
                      data.deliveries.recent.failed,
                  )}
                />
                <Detail
                  label={`Delivered (${period})`}
                  value={data.deliveries.recent.delivered}
                />
                <Detail
                  label={`Failed / uncertain (${period})`}
                  value={`${data.deliveries.recent.failed} / ${data.deliveries.recent.ambiguous}`}
                />
                <Detail
                  label="Queue-to-delivery median / p95"
                  value={`${duration(data.deliveries.recent.median_seconds)} / ${duration(data.deliveries.recent.p95_seconds)}`}
                />
                <Detail
                  label="Latency samples (holds excluded)"
                  value={data.deliveries.recent.latency_samples}
                />
                {data.deliveries.recent.delivered_after_hold > 0 && (
                  <Detail
                    label="Delivered after quiet-hours hold"
                    value={data.deliveries.recent.delivered_after_hold}
                  />
                )}
              </>
            ) : (
              <Detail label="Delivery state" value="Unknown" />
            )}
          </DetailCard>
        </div>
      </section>
    </Fragment>
  );
}

function DetailCard({
  id,
  title,
  status,
  children,
}: {
  id: string;
  title: string;
  status: HealthState;
  children: ReactNode;
}) {
  return (
    <article id={id} className="card system-detail-card">
      <div className="system-detail-heading">
        <h3>{title}</h3>
        <span className={`badge ${healthClass(status)}`}>{status}</span>
      </div>
      <dl>{children}</dl>
    </article>
  );
}

function Detail({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}
