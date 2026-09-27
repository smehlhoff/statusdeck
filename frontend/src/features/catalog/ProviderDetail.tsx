import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useState, type ReactNode } from "react";
import { ReliabilityHistory } from "./ReliabilityHistory";
import { ProviderComponents } from "./ProviderComponents";
import { Link, useParams } from "react-router-dom";
import { api } from "../../api/client";
import { MaintenanceWindow } from "../../components/MaintenanceWindow";
import {
  invalidateMonitoringState,
  LIVE_DATA_REFRESH_INTERVAL_MS,
  queryKeys,
} from "../../api/queries";
import type { CatalogDetail, Incident } from "../../api/types";
import { SourceButton } from "../../components/SourceButton";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { useToast } from "../../components/toastContext";
import { humanizeIdentifier, maintenanceTiming } from "../../utils/display";

function StatusHeading({ label, help }: { label: string; help: ReactNode }) {
  const tooltipId = useId();
  return (
    <h2 className="field-help-label">
      <span>{label}</span>
      <button
        className="field-help-button"
        type="button"
        aria-label={`About ${label}`}
        aria-describedby={tooltipId}
      >
        ?
      </button>
      <span className="field-help-tooltip" id={tooltipId} role="tooltip">
        {help}
      </span>
    </h2>
  );
}

export function ProviderDetail() {
  const { id } = useParams<{ id: string }>();
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const [confirmUnsubscribe, setConfirmUnsubscribe] = useState(false);

  const detail = useQuery({
    queryKey: queryKeys.catalogDetail(id),
    queryFn: ({ signal }) =>
      api<CatalogDetail>(`/api/v1/catalog/providers/${id}`, { signal }),
    enabled: Boolean(id),
    refetchInterval: LIVE_DATA_REFRESH_INTERVAL_MS,
    refetchIntervalInBackground: false,
  });

  useEffect(() => {
    document.title = `StatusDeck · ${detail.data?.name || "Provider details"}`;
  }, [id, detail.data?.name]);

  const deleteMonitor = useMutation({
    mutationFn: (monitorId: string) =>
      api<void>(`/api/v1/monitors/${monitorId}`, { method: "DELETE" }),
    onSuccess: () => {
      notify("Provider unsubscribed.");
      setConfirmUnsubscribe(false);
      void invalidateMonitoringState(queryClient);
    },
    onError: (error) => notify(error.message, "error"),
  });

  if (detail.isLoading)
    return <LoadingSkeleton label="Loading provider details" rows={4} />;
  if (!detail.data)
    return (
      <EmptyState
        title="Provider unavailable"
        description="The provider details could not be loaded. Try again or return to the catalog."
        error
        action={
          <div className="card-actions">
            <button
              className="button ghost"
              onClick={() => void detail.refetch()}
            >
              Retry
            </button>
            <Link className="button ghost" to="/catalog">
              Browse providers
            </Link>
          </div>
        }
      />
    );
  const provider = detail.data;
  const monitor = provider.monitor;
  const currentIncidents = provider.active_incidents.filter(
    (incident) => incident.kind === "incident",
  );
  const maintenanceNow = provider.active_incidents.filter(
    (incident) =>
      incident.kind === "maintenance" &&
      maintenanceTiming(incident.planned_start_at, incident.planned_end_at) ===
        "Scheduled window",
  );
  const upcomingMaintenance = provider.active_incidents.filter(
    (incident) =>
      incident.kind === "maintenance" &&
      maintenanceTiming(incident.planned_start_at, incident.planned_end_at) ===
        "Upcoming",
  );
  const unconfirmedMaintenance = provider.active_incidents.filter(
    (incident) =>
      incident.kind === "maintenance" &&
      !["Upcoming", "Scheduled window"].includes(
        maintenanceTiming(incident.planned_start_at, incident.planned_end_at),
      ),
  );

  return (
    <>
      <div className="page-heading">
        <div>
          <h1>{provider.name}</h1>
          <p className="muted">
            Provider health and monitoring freshness are independent signals.
          </p>
          <p className="muted">
            Source: {provider.source_adapter} · Last checked{" "}
            <RelativeDateTime
              value={provider.last_success_at}
              fallback="never"
            />
          </p>
        </div>
        <div className="heading-actions">
          {monitor?.enabled && (
            <button
              className="button danger"
              disabled={deleteMonitor.isPending}
              onClick={() => {
                deleteMonitor.reset();
                setConfirmUnsubscribe(true);
              }}
            >
              {deleteMonitor.isPending ? "Unsubscribing…" : "Unsubscribe"}
            </button>
          )}
          <SourceButton href={provider.official_url} />
        </div>
      </div>
      {detail.isError && (
        <p className="alert error" role="alert">
          Refresh failed. Showing the last provider update.{" "}
          <button
            className="button ghost"
            disabled={detail.isFetching}
            onClick={() => void detail.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      {deleteMonitor.isError && (
        <p className="alert error" role="alert">
          {deleteMonitor.error.message}
        </p>
      )}
      {confirmUnsubscribe && monitor && (
        <ConfirmDialog
          title={`Unsubscribe from ${provider.name}?`}
          message="StatusDeck will stop monitoring this provider's configured coverage."
          confirmLabel="Unsubscribe"
          pending={deleteMonitor.isPending}
          error={deleteMonitor.error?.message}
          onConfirm={() => deleteMonitor.mutate(monitor.id)}
          onClose={() => setConfirmUnsubscribe(false)}
        />
      )}
      <section
        className="incident-detail-grid provider-status-grid"
        aria-label="Provider status"
      >
        <div className="card incident-detail-card provider-status-card">
          <StatusHeading
            label="Configured coverage"
            help="Combined health for the coverage configured in StatusDeck, including your selected components. This can differ from the provider-wide status."
          />
          <strong>
            {provider.current_status
              ? humanizeIdentifier(provider.current_status)
              : "not checked"}
          </strong>
        </div>
        <div className="card incident-detail-card provider-status-card">
          <StatusHeading
            label="Provider-wide status"
            help="Overall health from the provider’s full status feed. This may include products or regions outside your configured coverage."
          />
          <strong>
            {provider.provider_status
              ? humanizeIdentifier(provider.provider_status)
              : "not checked"}
          </strong>
        </div>
        <div className="card incident-detail-card provider-status-card">
          <StatusHeading
            label="Monitoring"
            help="Freshness of status feed checks. Fresh means a recent successful check; delayed, stale or failing means updates may be behind. Not monitored means you have no active subscription."
          />
          <strong>
            {monitor?.enabled
              ? humanizeIdentifier(provider.freshness)
              : "not monitored"}
          </strong>
        </div>
        <div className="card incident-detail-card provider-status-card">
          <StatusHeading
            label="Active incidents"
            help={
              <>
                Open incidents and incidents awaiting confirmed resolution
                within this provider’s catalog coverage.
                <br />
                <br />
                Maintenance is excluded; the count can include components you
                have not selected.
              </>
            }
          />
          <strong>
            {
              provider.active_incidents.filter(
                (incident) => incident.kind === "incident",
              ).length
            }
          </strong>
        </div>
      </section>
      <section className="card provider-detail-section provider-events-section">
        {provider.recent_poll_error && (
          <p className="alert error" role="alert">
            Recent polling failure: {provider.recent_poll_error}
          </p>
        )}
        <div className="provider-event-groups">
          <ProviderEventGroup
            title="Current incidents"
            events={currentIncidents}
            emptyMessage="No current incidents"
          />
          <ProviderEventGroup
            title="Scheduled maintenance now"
            events={maintenanceNow}
            emptyMessage="No maintenance happening now"
          />
          <ProviderEventGroup
            title="Upcoming maintenance"
            events={upcomingMaintenance}
            emptyMessage="No upcoming maintenance"
          />
          <ProviderEventGroup
            title="Timing unavailable or ended"
            events={unconfirmedMaintenance}
            emptyMessage="No maintenance awaiting timing confirmation"
          />
        </div>
      </section>
      <ReliabilityHistory
        key={provider.id}
        providerId={provider.id}
        components={provider.components}
        monitored={Boolean(monitor?.enabled)}
      />
      <ProviderComponents key={provider.id} components={provider.components} />
    </>
  );
}

function ProviderEventGroup({
  title,
  events,
  emptyMessage,
}: {
  title: string;
  events: Incident[];
  emptyMessage: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const visibleEvents = expanded ? events : events.slice(0, 8);
  return (
    <article className="provider-event-group">
      <div className="provider-event-group-heading">
        <h3>{title}</h3>
        <span
          className="badge neutral provider-event-count"
          aria-label={`${events.length} ${events.length === 1 ? "event" : "events"}`}
        >
          {events.length}
        </span>
      </div>
      {events.length > 0 ? (
        <ul className="plain-list">
          {visibleEvents.map((incident) => (
            <li key={incident.id}>
              <Link to={`/incidents/${incident.id}`}>{incident.title}</Link>
              <div className="badge-row">
                <span className={`badge ${incident.severity}`}>
                  {incident.severity}
                </span>
                <span className="badge neutral">
                  {incident.kind === "maintenance"
                    ? maintenanceTiming(
                        incident.planned_start_at,
                        incident.planned_end_at,
                      )
                    : humanizeIdentifier(incident.lifecycle)}
                </span>
              </div>
              {incident.kind === "maintenance" && (
                <MaintenanceWindow
                  className="muted maintenance-window"
                  startAt={incident.planned_start_at}
                  endAt={incident.planned_end_at}
                />
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted provider-event-empty">{emptyMessage}</p>
      )}
      {events.length > 8 && (
        <button
          className="button ghost provider-event-toggle"
          type="button"
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded ? "Show fewer" : `Show all ${events.length}`}
        </button>
      )}
    </article>
  );
}
