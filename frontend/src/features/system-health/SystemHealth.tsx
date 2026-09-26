import { Link } from "react-router-dom";
import { DataMetrics } from "./DataMetrics";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { StatusBadge } from "../../components/StatusBadge";
import { useToast } from "../../components/toastContext";
import type {
  Delivery,
  DeliveryAttempt,
  DeliveryPage,
  Monitor,
  Source,
  SystemSummary,
  SystemWindow,
} from "../../api/types";
import { formatDuration, humanizeIdentifier } from "../../utils/display";
import { RuntimeOverview } from "./RuntimeOverview";
import {
  snapshotState,
  SYSTEM_REFRESH_MS as REFRESH_INTERVAL_MS,
  useDiagnosticClock,
} from "./diagnosticState";

const RECENT_DELIVERY_LIMIT = 50;
const RESEND_REFRESH_MS = 2_000;
const RESEND_FAST_REFRESH_WINDOW_MS = 30_000;

function deliveryStatusClass(status: string): string {
  switch (status) {
    case "delivered":
      return "operational";
    case "failed":
      return "major_outage";
    case "ambiguous":
    case "retrying":
      return "warning";
    case "suppressed":
      return "neutral";
    default:
      return "neutral";
  }
}

function pollOutcomeLabel(outcome: string | null): string {
  const label = humanizeIdentifier(outcome ?? "pending");
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export function SystemHealth() {
  const now = useDiagnosticClock();
  const [period, setPeriod] = useState<SystemWindow>("24h");
  const sources = useQuery({
    queryKey: queryKeys.sources,
    queryFn: ({ signal }) =>
      api<Source[]>("/api/v1/system/sources", { signal }),
    refetchInterval: REFRESH_INTERVAL_MS,
  });
  const summary = useQuery({
    queryKey: [...queryKeys.systemSummary, period],
    queryFn: ({ signal }) =>
      api<SystemSummary>(`/api/v1/system/summary?window=${period}`, { signal }),
    refetchInterval: REFRESH_INTERVAL_MS,
  });
  const monitors = useQuery({
    queryKey: queryKeys.monitors,
    queryFn: ({ signal }) => api<Monitor[]>("/api/v1/monitors", { signal }),
    refetchInterval: REFRESH_INTERVAL_MS,
  });
  const deliveries = useQuery({
    queryKey: queryKeys.recentDeliveries,
    queryFn: ({ signal }) =>
      api<DeliveryPage>(
        `/api/v1/system/deliveries?limit=${RECENT_DELIVERY_LIMIT}`,
        { signal },
      ),
    refetchInterval: (query) =>
      query.state.data?.items.some(
        (delivery) =>
          delivery.status === "pending" &&
          delivery.resend_requested_at !== null &&
          Date.now() - Date.parse(delivery.resend_requested_at) <
            RESEND_FAST_REFRESH_WINDOW_MS,
      )
        ? RESEND_REFRESH_MS
        : REFRESH_INTERVAL_MS,
  });

  if (
    summary.isError &&
    !summary.data &&
    sources.isError &&
    !sources.data &&
    deliveries.isError &&
    !deliveries.data
  ) {
    return (
      <EmptyState
        title="Diagnostics unavailable"
        description="The diagnostics backend could not be reached. Check the connection and try again."
        error
        action={
          <button
            className="button ghost"
            type="button"
            onClick={() => {
              void summary.refetch();
              void sources.refetch();
              void monitors.refetch();
              void deliveries.refetch();
            }}
          >
            Retry
          </button>
        }
      />
    );
  }

  const monitoredProviderIds = new Set(
    monitors.data
      ?.filter((monitor) => monitor.enabled)
      .map((monitor) => monitor.provider_id) ?? [],
  );
  const currentSources = sources.data?.filter((source) => source.enabled) ?? [];
  const activeSources = currentSources.filter((source) =>
    source.affected_providers.some((provider) =>
      monitoredProviderIds.has(provider.id),
    ),
  );
  const sourceState = snapshotState(sources, now);
  const monitorState = snapshotState(monitors, now);
  const deliveryState = snapshotState(deliveries, now);

  return (
    <>
      <div className="page-heading">
        <div>
          <h1>System</h1>
          <p className="muted">
            StatusDeck runtime, polling pipeline, and provider connectivity.
          </p>
        </div>
      </div>

      <RuntimeOverview
        summary={summary}
        sources={sources}
        monitors={monitors}
        now={now}
        window={period}
        onWindowChange={setPeriod}
      />

      <DataMetrics now={now} />

      <section id="provider-monitoring" className="card diagnostics-section">
        <div className="section-heading">
          <div>
            <h2>Provider monitoring</h2>
          </div>
          <span className="muted">
            {sourceState === "Healthy" && monitorState === "Healthy"
              ? `${activeSources.length} active · ${currentSources.length} catalog`
              : "Coverage unverified"}
          </span>
        </div>
        <SourceDiagnostics
          sources={sources.data}
          monitoredProviderIds={monitoredProviderIds}
          loading={sources.isLoading || monitors.isLoading}
          failed={
            sourceState === "Stale" ||
            sourceState === "Unavailable" ||
            monitorState === "Stale" ||
            monitorState === "Unavailable"
          }
        />
      </section>
      <DeliveryDiagnostics
        page={deliveries.data}
        loading={deliveries.isLoading}
        failed={deliveryState === "Stale" || deliveryState === "Unavailable"}
      />
    </>
  );
}

function freshnessLabel(source: Source): string {
  if (!source.enabled) return "Removed";
  if (source.freshness === "never_checked") return "Awaiting first check";
  return humanizeIdentifier(source.freshness);
}

function pollOutcomeDescription(source: Source): string {
  if (!source.enabled)
    return "This provider was removed from the catalog. Monitoring has stopped; its history is retained.";
  if (source.last_poll_outcome === "running")
    return "A provider check is in progress.";
  if (source.last_poll_outcome === "not_modified") {
    return "The provider responded successfully with no changes.";
  }
  if (source.last_poll_outcome === "success") {
    return "The latest provider check completed successfully.";
  }
  if (source.last_poll_outcome === null) {
    return "This source has not completed a check yet.";
  }
  const name =
    source.affected_providers.map((provider) => provider.name).join(", ") ||
    source.source_key;
  switch (source.last_poll_error_class) {
    case "parse":
    case "response_too_large":
      return `Unable to read ${name}’s status feed. Check its official status page; if this continues, contact your StatusDeck administrator.`;
    case "transport":
    case "http":
    case "rate_limit":
      return `Unable to retrieve ${name}’s status feed. StatusDeck will retry automatically. Check the provider’s official status page for current updates.`;
    default:
      return `Unable to update ${name}’s monitoring status. StatusDeck will retry automatically; contact your administrator if this continues.`;
  }
}

function SourceDiagnostics({
  sources,
  monitoredProviderIds,
  loading,
  failed,
}: {
  sources: Source[] | undefined;
  monitoredProviderIds: ReadonlySet<string>;
  loading: boolean;
  failed: boolean;
}) {
  const [selectedSourceId, setSelectedSourceId] = useState<string>();
  const selectedSource = sources?.find(
    (source) => source.id === selectedSourceId,
  );
  const orderedSources = [...(sources ?? [])].sort((left, right) => {
    const leftSubscribed = left.affected_providers.some((provider) =>
      monitoredProviderIds.has(provider.id),
    );
    const rightSubscribed = right.affected_providers.some((provider) =>
      monitoredProviderIds.has(provider.id),
    );
    return (
      Number(rightSubscribed) - Number(leftSubscribed) ||
      left.source_key.localeCompare(right.source_key)
    );
  });

  if (failed) {
    return (
      <p className="alert error">Source diagnostics could not be loaded.</p>
    );
  }
  if (loading)
    return <LoadingSkeleton label="Loading provider monitoring" inline />;
  if (!sources?.length)
    return (
      <EmptyState
        title="No provider sources"
        description="Provider sources will appear here after the catalog is configured."
        inline
      />
    );
  return (
    <>
      <div
        className="table-wrap source-table-wrap"
        role="region"
        aria-label="Provider monitoring diagnostics"
        tabIndex={0}
      >
        <table className="data-table responsive-data-table">
          <thead>
            <tr>
              <th>Provider</th>
              <th>Subscription</th>
              <th>Freshness</th>
              <th>Last successful check</th>
              <th>Last poll</th>
              <th>Next check</th>
              <th>Consecutive failures</th>
              <th>Failed status checks (24h)</th>
            </tr>
          </thead>
          <tbody>
            {orderedSources.map((source) => {
              const subscribedProviders = source.affected_providers.filter(
                (provider) => monitoredProviderIds.has(provider.id),
              );
              return (
                <tr key={source.id}>
                  <td data-label="Provider">
                    <button
                      className="event-id-button source-name-button"
                      type="button"
                      onClick={() => setSelectedSourceId(source.id)}
                    >
                      {source.source_key}
                    </button>
                    <small>
                      {source.adapter} ·{" "}
                      {source.affected_providers
                        .map((provider) => provider.name)
                        .join(", ") || "Catalog only"}
                    </small>
                  </td>
                  <td data-label="Subscription">
                    <span
                      className={`badge ${
                        subscribedProviders.length > 0
                          ? "operational"
                          : "neutral"
                      }`}
                    >
                      {subscribedProviders.length > 0
                        ? "Subscribed"
                        : "Not subscribed"}
                    </span>
                    {subscribedProviders.length > 0 &&
                      subscribedProviders.length <
                        source.affected_providers.length && (
                        <small>
                          {subscribedProviders
                            .map((provider) => provider.name)
                            .join(", ")}
                        </small>
                      )}
                  </td>
                  <td data-label="Freshness">
                    <span
                      className={`badge ${source.enabled ? source.freshness : "neutral"}`}
                      title={
                        !source.enabled
                          ? "This provider is no longer monitored."
                          : `Monitoring freshness: ${freshnessLabel(source)}.`
                      }
                    >
                      {freshnessLabel(source)}
                    </span>
                  </td>
                  <td data-label="Last successful check">
                    <RelativeDateTime value={source.last_success_at} />
                  </td>
                  <td data-label="Last poll">
                    {formatDuration(
                      source.last_poll_duration_ms === null
                        ? null
                        : source.last_poll_duration_ms / 1_000,
                    )}
                    {source.last_poll_http_status !== null
                      ? ` · HTTP ${source.last_poll_http_status}`
                      : ""}
                  </td>
                  <td data-label="Next check">
                    {source.enabled ? (
                      <RelativeDateTime value={source.next_poll_at} />
                    ) : (
                      "Not scheduled"
                    )}
                  </td>
                  <td data-label="Consecutive failures">
                    <button
                      className="event-id-button"
                      type="button"
                      aria-label={`Open diagnostics for ${source.source_key}; ${source.consecutive_failures} consecutive failures`}
                      onClick={() => setSelectedSourceId(source.id)}
                    >
                      {source.consecutive_failures}
                    </button>
                  </td>
                  <td data-label="Failed status checks (24h)">
                    {source.status_checks_24h === 0
                      ? "No checks in 24h"
                      : `${source.status_failures_24h} / ${source.status_checks_24h}`}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {selectedSource && (
        <SourceDetailDialog
          source={selectedSource}
          onClose={() => setSelectedSourceId(undefined)}
        />
      )}
    </>
  );
}

function SourceDetailDialog({
  source,
  onClose,
}: {
  source: Source;
  onClose: () => void;
}) {
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
      className="source-detail-dialog"
      aria-labelledby={titleId}
      onCancel={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="section-heading">
        <div>
          <h2 id={titleId}>{source.source_key}</h2>
        </div>
        <button
          className="button ghost compact"
          type="button"
          onClick={onClose}
        >
          Close
        </button>
      </div>
      <dl>
        <dt>Latest outcome</dt>
        <dd>{pollOutcomeLabel(source.last_poll_outcome)}</dd>
        <dt>Outcome detail</dt>
        <dd>{pollOutcomeDescription(source)}</dd>
        <dt>Duration</dt>
        <dd>
          {formatDuration(
            source.last_poll_duration_ms === null
              ? null
              : source.last_poll_duration_ms / 1_000,
          )}
        </dd>
        <dt>Last attempt</dt>
        <dd>
          <RelativeDateTime value={source.last_attempt_at} />
        </dd>
        <dt>Last successful check</dt>
        <dd>
          <RelativeDateTime value={source.last_success_at} />
        </dd>
        <dt>Consecutive failures</dt>
        <dd>{source.consecutive_failures}</dd>
        <dt>Failed status checks (24h)</dt>
        <dd>
          {source.status_checks_24h === 0
            ? "No checks in 24h"
            : `${source.status_failures_24h} / ${source.status_checks_24h}`}
        </dd>
        <dt>Next check</dt>
        <dd>
          {source.enabled ? (
            <RelativeDateTime value={source.next_poll_at} />
          ) : (
            "Not scheduled"
          )}
        </dd>
        <dt>Affected providers</dt>
        <dd>
          {source.affected_providers
            .map((provider) => provider.name)
            .join(", ") || "Catalog only"}
        </dd>
      </dl>
      <details className="diagnostic-technical-details">
        <summary>Technical details</summary>
        <dl>
          <dt>Adapter</dt>
          <dd>{source.adapter}</dd>
          <dt>Adapter version</dt>
          <dd>{source.last_poll_adapter_version ?? "—"}</dd>
          <dt>HTTP status</dt>
          <dd>{source.last_poll_http_status ?? "—"}</dd>
          <dt>Error class</dt>
          <dd>{source.last_poll_error_class ?? "—"}</dd>
          <dt>Error message</dt>
          <dd>{source.last_error ?? "None"}</dd>
        </dl>
      </details>
    </dialog>
  );
}

function DeliveryDiagnostics({
  page,
  loading,
  failed,
}: {
  page: DeliveryPage | undefined;
  loading: boolean;
  failed: boolean;
}) {
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const resend = useMutation({
    mutationFn: (id: string) =>
      api<{ delivery_id: string; status: string }>(
        `/api/v1/system/deliveries/${id}/resend`,
        { method: "POST" },
      ),
    onSuccess: async () => {
      notify("Event queued for resend.");
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.deliveries }),
        queryClient.invalidateQueries({ queryKey: queryKeys.systemSummary }),
        queryClient.invalidateQueries({ queryKey: queryKeys.dataMetrics }),
      ]);
    },
    onError: (error) => {
      notify(error.message, "error");
      void queryClient.invalidateQueries({ queryKey: queryKeys.deliveries });
    },
  });
  const [selectedDeliverySnapshot, setSelectedDeliverySnapshot] =
    useState<Delivery>();
  const selectedDelivery = selectedDeliverySnapshot
    ? (page?.items.find(
        (delivery) =>
          delivery.notification_event_id ===
            selectedDeliverySnapshot.notification_event_id &&
          delivery.id === selectedDeliverySnapshot.id,
      ) ?? selectedDeliverySnapshot)
    : undefined;

  return (
    <>
      <section id="delivery-history" className="card diagnostics-section">
        <div className="section-heading">
          <div>
            <h2>Delivery diagnostics</h2>
          </div>
        </div>
        {loading && <LoadingSkeleton label="Loading delivery history" inline />}
        {failed && (
          <p className="alert error">
            Delivery diagnostics could not be refreshed. Any visible records are
            from the last successful snapshot.
          </p>
        )}
        {page && page.items.length === 0 && (
          <EmptyState
            title="No notification events"
            description="There are no notification events on this page."
            inline
          />
        )}
        {page && page.items.length > 0 && (
          <div
            className="table-wrap delivery-table-wrap"
            role="region"
            aria-label="Recent notification event and delivery history"
            tabIndex={0}
          >
            <table className="data-table responsive-data-table">
              <thead>
                <tr>
                  <th>Event</th>
                  <th>Incident</th>
                  <th>Destination</th>
                  <th>Event ID</th>
                  <th>Status</th>
                  <th>Created</th>
                  <th>Attempts</th>
                  <th>Last error</th>
                  <th>Actions</th>
                </tr>
              </thead>
              <tbody>
                {page.items.map((delivery) => (
                  <tr
                    key={`${delivery.notification_event_id}:${delivery.id ?? "suppressed"}`}
                  >
                    <td data-label="Event">{delivery.event_type}</td>
                    <td data-label="Incident">
                      {delivery.incident_id ? (
                        <Link to={`/incidents/${delivery.incident_id}`}>
                          View incident
                        </Link>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td data-label="Destination">
                      {delivery.channel_name ?? "—"}
                    </td>
                    <td data-label="Event ID">
                      <button
                        className="event-id-button"
                        type="button"
                        onClick={() => setSelectedDeliverySnapshot(delivery)}
                      >
                        {delivery.notification_event_id}
                      </button>
                    </td>
                    <td data-label="Status">
                      <span
                        className={`badge ${deliveryStatusClass(delivery.status)}`}
                      >
                        {humanizeIdentifier(delivery.status)}
                      </span>
                      {delivery.status === "held" && delivery.quiet_until && (
                        <div className="muted">
                          Quiet hours until{" "}
                          <RelativeDateTime value={delivery.quiet_until} />
                        </div>
                      )}
                    </td>
                    <td data-label="Created">
                      <RelativeDateTime value={delivery.created_at} />
                    </td>
                    <td data-label="Attempts">
                      {delivery.id ? (
                        <button
                          className="event-id-button"
                          type="button"
                          aria-label={`View ${delivery.attempt_count} delivery attempts`}
                          onClick={() => setSelectedDeliverySnapshot(delivery)}
                        >
                          {delivery.attempt_count}
                        </button>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td data-label="Last error">
                      {delivery.last_error ?? "—"}
                    </td>
                    <td data-label="Actions">
                      {delivery.id ? (
                        <button
                          className="button ghost compact"
                          type="button"
                          disabled={!delivery.can_resend || resend.isPending}
                          aria-label={`Resend ${delivery.event_type} event`}
                          title={
                            delivery.can_resend
                              ? "Resend this event to its original channel now"
                              : "Resend requires a completed delivery, an enabled channel, and no queued delivery for this event"
                          }
                          onClick={() => {
                            if (!resend.isPending) resend.mutate(delivery.id!);
                          }}
                        >
                          {resend.isPending && resend.variables === delivery.id
                            ? "Queuing…"
                            : "Resend"}
                        </button>
                      ) : (
                        "—"
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      {selectedDelivery && (
        <EventPayloadDialog
          delivery={selectedDelivery}
          onClose={() => setSelectedDeliverySnapshot(undefined)}
        />
      )}
    </>
  );
}

function EventPayloadDialog({
  delivery,
  onClose,
}: {
  delivery: Delivery;
  onClose: () => void;
}) {
  const attempts = useQuery({
    queryKey: [
      ...queryKeys.deliveries,
      "attempts",
      delivery.id ?? "suppressed",
      delivery.attempt_count,
    ],
    queryFn: ({ signal }) =>
      delivery.id
        ? api<DeliveryAttempt[]>(
            `/api/v1/system/deliveries/${delivery.id}/attempts`,
            { signal },
          )
        : Promise.resolve([]),
    enabled: delivery.id !== null,
  });
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
      className="event-payload-dialog"
      aria-labelledby={titleId}
      onCancel={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="event-payload-heading">
        <div>
          <h2 id={titleId}>Delivery details</h2>
          <div className="event-payload-meta">
            <span className="delivery-event-name">
              {humanizeIdentifier(delivery.event_type.replaceAll(".", " "))}
            </span>
            <StatusBadge
              value={delivery.status}
              tone={deliveryStatusClass(delivery.status)}
            />
          </div>
        </div>
        <button
          className="button ghost compact"
          type="button"
          onClick={onClose}
          autoFocus
        >
          Close
        </button>
      </div>
      <div className="delivery-details-body">
        <section
          className="delivery-attempts"
          aria-labelledby={`${titleId}-attempts`}
        >
          <div className="delivery-section-heading">
            <h3 id={`${titleId}-attempts`}>
              Attempts <span>({delivery.attempt_count})</span>
            </h3>
          </div>
          {attempts.isLoading && (
            <LoadingSkeleton label="Loading delivery attempts" inline />
          )}
          {attempts.isError && (
            <div className="delivery-history-error" role="alert">
              <p>Attempt history could not be loaded.</p>
              <button
                className="button ghost compact"
                type="button"
                onClick={() => void attempts.refetch()}
                disabled={attempts.isFetching}
              >
                Retry
              </button>
            </div>
          )}
          {attempts.data?.length === 0 && (
            <p className="delivery-history-empty">No send attempts yet.</p>
          )}
          {attempts.data && attempts.data.length > 0 && (
            <ol
              className="delivery-attempt-list"
              aria-label="Attempts, newest first"
            >
              {attempts.data.map((attempt, index, history) => (
                <li key={attempt.id}>
                  <div className="delivery-attempt-heading">
                    <span className="delivery-attempt-number">
                      Attempt {history.length - index}
                    </span>
                    {attempt.outcome !== "delivered" && (
                      <span>{humanizeIdentifier(attempt.outcome)}</span>
                    )}
                  </div>
                  <div className="delivery-attempt-time">
                    <RelativeDateTime value={attempt.attempted_at} />
                  </div>
                  <div className="delivery-http-status">
                    {attempt.response_status !== null && (
                      <code>HTTP {attempt.response_status}</code>
                    )}
                  </div>
                  {attempt.error_message && (
                    <p className="delivery-attempt-error">
                      {attempt.error_message}
                    </p>
                  )}
                </li>
              ))}
            </ol>
          )}
        </section>
        <div className="delivery-payload-section">
          <dl className="delivery-overview">
            <dt>Destination</dt>
            <dd>{delivery.channel_name ?? "No delivery destination"}</dd>
            {delivery.suppression_reason && (
              <>
                <dt>Suppression reason</dt>
                <dd>{delivery.suppression_reason}</dd>
              </>
            )}
            <dt>Event ID</dt>
            <dd>
              <code>{delivery.notification_event_id}</code>
            </dd>
          </dl>
          <pre className="event-payload-json">
            {JSON.stringify(delivery.payload, null, 2)}
          </pre>
        </div>
      </div>
    </dialog>
  );
}
