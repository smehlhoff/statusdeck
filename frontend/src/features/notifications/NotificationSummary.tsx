import { useQuery } from "@tanstack/react-query";
import { Fragment, useEffect } from "react";
import { Link, useParams } from "react-router-dom";
import { ApiFailure, api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { NotificationSummary as SummaryData } from "../../api/types";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { MetricCard } from "../../components/MetricCard";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { humanizeIdentifier } from "../../utils/display";

export function NotificationSummary() {
  const { id } = useParams<{ id: string }>();
  const query = useQuery({
    queryKey: queryKeys.notificationSummary(id),
    queryFn: ({ signal }) =>
      api<SummaryData>(`/api/v1/notifications/summaries/${id}`, { signal }),
    enabled: Boolean(id),
  });
  const title =
    query.data?.payload.title.replace(
      /^Quiet-hours summary:/i,
      "Quiet-Hours Summary:",
    ) ?? "Quiet-Hours Summary";

  useEffect(() => {
    document.title = `StatusDeck · ${title}`;
  }, [id, title]);

  if (query.isLoading)
    return <LoadingSkeleton label="Loading quiet-hours summary" rows={4} />;
  if (!query.data) {
    const missing =
      query.error instanceof ApiFailure &&
      (query.error.status === 404 || query.error.status === 400);
    return (
      <EmptyState
        title={missing ? "Summary not found" : "Summary unavailable"}
        description={
          missing
            ? "This quiet-hours summary could not be found. Check the notification link."
            : "The summary could not be loaded. Try again."
        }
        error
        action={
          <div className="card-actions">
            {!missing && (
              <button
                className="button ghost"
                onClick={() => void query.refetch()}
              >
                Retry
              </button>
            )}
            <Link className="button ghost" to="/notifications">
              Notifications
            </Link>
          </div>
        }
      />
    );
  }

  const { payload: summary } = query.data;
  return (
    <>
      <div className="page-heading analytics-heading notification-summary-heading">
        <div>
          <div className="field-help-label">
            <h1>{title}</h1>
          </div>
          <p className="muted">
            Events held during quiet hours, with their status at the time this
            summary was saved.
          </p>
        </div>
        <div className="analytics-heading-actions">
          <Link className="button ghost" to="/notifications">
            Back to notifications
          </Link>
        </div>
      </div>
      <section
        className="analytics-summary-grid notification-summary-grid"
        aria-label="Summary details"
      >
        <MetricCard
          label="Held events"
          value={summary.event_count}
          note="Notifications paused during quiet hours"
        />
        <MetricCard
          label="Quiet hours ended"
          value={<RelativeDateTime value={summary.quiet_until} />}
          note="End of the scheduled quiet-hours window"
        />
      </section>
      <section className="card" aria-labelledby="summary-entries-heading">
        <div className="section-heading">
          <div>
            <h2 id="summary-entries-heading">
              What happened during quiet hours
            </h2>
            <p className="muted">
              Open an incident or provider to see its latest status.
            </p>
          </div>
          <span className="badge neutral">
            {summary.entries.length}{" "}
            {summary.entries.length === 1 ? "entry" : "entries"}
          </span>
        </div>
        {summary.entries.length === 0 && (
          <EmptyState
            title="No summary entries"
            description="There are no saved entries in this summary."
            inline
          />
        )}
        {summary.entries.length > 0 && (
          <div
            className="table-wrap source-table-wrap"
            role="region"
            aria-label="Summary entries"
            tabIndex={0}
          >
            <table className="data-table responsive-data-table notification-summary-table">
              <thead>
                <tr>
                  <th scope="col">Provider</th>
                  <th scope="col">Incident or provider</th>
                  <th scope="col">Saved status</th>
                  <th scope="col">Started</th>
                  <th scope="col">Resolved</th>
                  <th scope="col">Last event</th>
                </tr>
              </thead>
              <tbody>
                {summary.entries.map((entry, index) => {
                  let destination: string | undefined;
                  if (entry.event_type.startsWith("incident.")) {
                    destination = `/incidents/${entry.entity_id}`;
                  } else if (entry.event_type === "provider.status_changed") {
                    destination = `/catalog/${entry.entity_id}`;
                  }
                  return (
                    <tr key={`${entry.entity_id}-${index}`}>
                      <td data-label="Provider">
                        {entry.providers.length > 0
                          ? entry.providers.map((provider, providerIndex) => (
                              <Fragment key={provider.id}>
                                {providerIndex > 0 && ", "}
                                <Link to={`/catalog/${provider.id}`}>
                                  {provider.name}
                                </Link>
                              </Fragment>
                            ))
                          : "Not reported"}
                      </td>
                      <td data-label="Incident or provider">
                        <strong>
                          {destination ? (
                            <Link
                              className="event-id-button source-name-button"
                              to={destination}
                            >
                              {entry.title}
                            </Link>
                          ) : (
                            entry.title
                          )}
                        </strong>
                      </td>
                      <td data-label="Saved status">
                        <span className="badge neutral">
                          {humanizeIdentifier(entry.status)}
                        </span>
                      </td>
                      <td data-label="Started">
                        <RelativeDateTime
                          value={entry.started_at}
                          fallback="Not reported"
                        />
                      </td>
                      <td data-label="Resolved">
                        <RelativeDateTime
                          value={entry.resolved_at}
                          fallback="Not reported"
                        />
                      </td>
                      <td data-label="Last event">
                        <RelativeDateTime value={entry.last_event_at} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
        {summary.omitted_entries > 0 && (
          <p className="muted">
            {summary.omitted_entries} additional entr
            {summary.omitted_entries === 1 ? "y was" : "ies were"} omitted from
            the saved summary, which includes up to 50 entries.
          </p>
        )}
      </section>
    </>
  );
}
