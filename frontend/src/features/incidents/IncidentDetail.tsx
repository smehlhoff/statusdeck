import { useQuery } from "@tanstack/react-query";
import { Fragment, useEffect, useId } from "react";
import { Link, useLocation, useParams } from "react-router-dom";
import { api } from "../../api/client";
import { LIVE_DATA_REFRESH_INTERVAL_MS, queryKeys } from "../../api/queries";
import type {
  IncidentDetail as IncidentDetailData,
  IncidentUpdate,
} from "../../api/types";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { SourceButton } from "../../components/SourceButton";
import { ProviderContent } from "../../components/ProviderContent";
import { IncidentComments } from "./IncidentComments";
import { incidentUpdateStatus } from "./updateStatus";
import {
  formatDuration,
  formatProviderPhase,
  humanizeIdentifier,
  maintenanceTiming,
  providerText,
} from "../../utils/display";
import { BookmarkButton } from "./BookmarkButton";
import { CopyIncidentLinkButton } from "./CopyIncidentLinkButton";

interface IncidentUpdateGroup {
  updates: IncidentUpdate[];
}

function incidentUpdateSignature(update: IncidentUpdate): string {
  return JSON.stringify({
    status: update.status,
    body: providerText(update.body),
    synthesized: update.synthesized,
    components: update.components.map((component) => component.id).sort(),
    scopes: update.scopes
      .map((scope) => [scope.type, scope.upstream_id, scope.original_status])
      .sort(),
  });
}

function groupIncidentUpdates(
  updates: IncidentUpdate[],
): IncidentUpdateGroup[] {
  return updates.reduce<IncidentUpdateGroup[]>((groups, update) => {
    const previous = groups.at(-1);
    const previousUpdate = previous?.updates.at(-1);
    if (
      previous &&
      previousUpdate &&
      incidentUpdateSignature(previousUpdate) ===
        incidentUpdateSignature(update)
    ) {
      previous.updates.push(update);
    } else {
      groups.push({ updates: [update] });
    }
    return groups;
  }, []);
}

function wasEdited(update: IncidentUpdate): boolean {
  if (!update.created_at || !update.updated_at) return false;
  return (
    Math.abs(Date.parse(update.updated_at) - Date.parse(update.created_at)) >=
    1_000
  );
}

function sentenceCase(value: string): string {
  const text = value.trim().toLowerCase();
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function metadataValue(key: string, value: unknown): string | number {
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value === "string")
    return key.endsWith("_id") ? value : sentenceCase(value);
  if (typeof value === "number") return value;
  if (value === null) return "—";
  return JSON.stringify(value) ?? String(value);
}

export function IncidentDetail() {
  const { id } = useParams<{ id: string }>();
  const location = useLocation();
  const navigation = location.state as {
    incidentSearch?: string;
    bookmarkSearch?: string;
    myCommentsSearch?: string;
  } | null;
  const incidentSearch = navigation?.incidentSearch;
  const myCommentsSearch = navigation?.myCommentsSearch;
  const bookmarkSearch = navigation?.bookmarkSearch;
  const incidentFeed = incidentSearch
    ? `/incidents?${incidentSearch}`
    : "/incidents";
  const query = useQuery({
    queryKey: queryKeys.incidentDetail(id),
    queryFn: ({ signal }) =>
      api<IncidentDetailData>(`/api/v1/incidents/${id}`, { signal }),
    enabled: Boolean(id),
    refetchInterval: LIVE_DATA_REFRESH_INTERVAL_MS,
    refetchIntervalInBackground: false,
  });
  useEffect(() => {
    document.title = `StatusDeck · ${query.data?.incident.title || "Incident details"}`;
  }, [id, query.data?.incident.title]);

  if (query.isLoading)
    return <LoadingSkeleton label="Loading incident details" rows={5} />;
  if (!query.data)
    return (
      <EmptyState
        title="Incident unavailable"
        description="The incident details could not be loaded. Try again or return to the incident feed."
        error
        action={
          <div className="card-actions">
            <button
              className="button ghost"
              onClick={() => void query.refetch()}
            >
              Retry
            </button>
            <Link className="button ghost" to={incidentFeed}>
              Back to incidents
            </Link>
          </div>
        }
      />
    );
  const { incident, providers, components, scopes, updates } = query.data;
  const componentNames =
    components.length > 0
      ? components.map((component) => component.name)
      : scopes
          .filter((scope) => scope.type === "component")
          .map((scope) => scope.name);
  const instances = scopes.filter((scope) => scope.type === "instance");
  let reportedDuration =
    incident.duration_seconds === null
      ? "Duration unknown"
      : formatDuration(incident.duration_seconds);
  if (
    incident.duration_seconds === null &&
    incident.kind === "maintenance" &&
    incident.lifecycle !== "resolved" &&
    incident.original_phase.toLowerCase() === "scheduled" &&
    maintenanceTiming(incident.planned_start_at, incident.planned_end_at) ===
      "Upcoming"
  ) {
    reportedDuration = "Not started";
  }
  const updateGroups = groupIncidentUpdates(updates).reverse();
  const repeatedUpdates = updates.length - updateGroups.length;
  return (
    <>
      <div className="page-heading incident-heading">
        <div>
          <h1>{incident.title}</h1>
          <div className="badge-row">
            <span className={"badge " + incident.severity}>
              {incident.severity}
            </span>
            <span className="badge neutral">
              {humanizeIdentifier(incident.lifecycle)}
            </span>
            <span className="badge neutral">{incident.kind}</span>
            <span
              className={`badge ${incident.within_provider_scope ? "neutral" : "warning"}`}
            >
              {incident.within_provider_scope
                ? "Within provider scope"
                : "Outside provider scope"}
            </span>
            {incident.kind === "maintenance" &&
              incident.lifecycle !== "resolved" && (
                <span className="badge neutral">
                  {maintenanceTiming(
                    incident.planned_start_at,
                    incident.planned_end_at,
                  )}
                </span>
              )}
          </div>
        </div>
        <div className="incident-actions">
          {myCommentsSearch !== undefined && (
            <Link
              className="button ghost"
              to={`/my-comments?${myCommentsSearch}`}
            >
              Back to My Comments
            </Link>
          )}
          {bookmarkSearch !== undefined && (
            <Link className="button ghost" to={`/bookmarks?${bookmarkSearch}`}>
              Back to bookmarks
            </Link>
          )}
          <BookmarkButton
            incidentId={incident.id}
            bookmarked={incident.bookmarked}
          />
          <CopyIncidentLinkButton incidentId={incident.id} />
          {incident.official_url && (
            <SourceButton href={incident.official_url} />
          )}
        </div>
      </div>
      {query.isError && (
        <p className="alert error" role="alert">
          Refresh failed. Showing the last incident update.{" "}
          <button
            className="button ghost"
            disabled={query.isFetching}
            onClick={() => void query.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      <div className="incident-detail-grid" aria-label="Incident details">
        <section className="card incident-detail-card">
          <h2>Details</h2>
          <dl className="incident-detail-list">
            <div>
              <DetailTerm
                label="Provider"
                help="The provider that published this incident."
              />
              <dd>
                {providers.length > 0
                  ? providers.map((provider, index) => (
                      <Fragment key={provider.id}>
                        {index > 0 && ", "}
                        <Link to={`/catalog/${provider.id}`}>
                          {provider.name}
                        </Link>
                      </Fragment>
                    ))
                  : "Not reported"}
              </dd>
            </div>
            <div>
              <DetailTerm
                label="Provider reference"
                help="The incident identifier assigned by the provider."
              />
              <dd>{incident.upstream_incident_id}</dd>
            </div>
            <div>
              <DetailTerm
                label="Component"
                help="Provider components reported as affected by this incident."
              />
              <dd>
                {componentNames.length > 0
                  ? componentNames.join(", ")
                  : "Not reported"}
              </dd>
            </div>
            {(instances.length > 0 ||
              providers.some((provider) => provider.slug === "salesforce")) && (
              <div>
                <DetailTerm
                  label="Impacted instances"
                  help="Instances reported as impacted by the provider for this incident or maintenance."
                />
                <dd>
                  {instances.length > 0
                    ? instances.map((instance) => instance.name).join(", ")
                    : "Not reported"}
                </dd>
              </div>
            )}
            <div>
              <DetailTerm
                label="Provider phase"
                help="The incident phase currently reported by the provider."
              />
              <dd className="provider-phase">
                {formatProviderPhase(incident.original_phase)}
              </dd>
            </div>
            {incident.original_impact && (
              <div>
                <DetailTerm
                  label="Provider impact"
                  help="The provider's original impact classification before StatusDeck normalization."
                />
                <dd>
                  {sentenceCase(humanizeIdentifier(incident.original_impact))}
                </dd>
              </div>
            )}
            {Object.entries(incident.provider_metadata).map(([key, value]) => (
              <div key={key}>
                <DetailTerm
                  label={sentenceCase(humanizeIdentifier(key))}
                  help="Additional incident metadata supplied by the provider."
                />
                <dd>{metadataValue(key, value)}</dd>
              </div>
            ))}
          </dl>
        </section>
        {incident.kind === "maintenance" && (
          <section className="card incident-detail-card">
            <h2>Scheduled maintenance window</h2>
            {incident.maintenance_uncertain && (
              <p className="muted">
                The scheduled window has ended, but completion is unconfirmed.
                Current maintenance status is uncertain.
              </p>
            )}
            <dl className="incident-detail-list">
              <div>
                <dt>Planned start</dt>
                <DetailDate value={incident.planned_start_at} />
              </div>
              <div>
                <dt>Planned end</dt>
                <DetailDate value={incident.planned_end_at} />
              </div>
            </dl>
          </section>
        )}
        <section className="card incident-detail-card">
          <h2>Provider activity</h2>
          <dl className="incident-detail-list">
            <div>
              <DetailTerm
                label="Reported duration"
                help="Incident duration runs from the reported start (or first publication) to resolution, or now while ongoing. Future scheduled maintenance shows Not started; its planned window is not actual impact. Missing or inconsistent timestamps leave duration unknown."
              />
              <dd>{reportedDuration}</dd>
            </div>
            <div>
              <DetailTerm
                label="Created"
                help="When the provider says the incident was created."
              />
              <DetailDate value={incident.provider_created_at} />
            </div>
            {incident.provider_started_at && (
              <div>
                <DetailTerm
                  label="Started"
                  help="When the provider says the provider impact began."
                />
                <DetailDate value={incident.provider_started_at} />
              </div>
            )}
            <div>
              <DetailTerm
                label="Updated"
                help="When the provider last changed the incident."
              />
              <DetailDate value={incident.provider_updated_at} />
            </div>
            {incident.provider_monitoring_at && (
              <div>
                <DetailTerm
                  label="Monitoring began"
                  help="When the provider moved the incident into a monitoring phase."
                />
                <DetailDate value={incident.provider_monitoring_at} />
              </div>
            )}
            {incident.provider_resolved_at && (
              <div>
                <DetailTerm
                  label="Resolved"
                  help="When the provider marked the incident as resolved."
                />
                <DetailDate value={incident.provider_resolved_at} />
              </div>
            )}
          </dl>
        </section>
        <section className="card incident-detail-card">
          <h2>StatusDeck activity</h2>
          <dl className="incident-detail-list">
            <div>
              <DetailTerm
                label="First observed"
                help="When StatusDeck first detected this incident."
              />
              <DetailDate value={incident.first_observed_at} />
            </div>
            <div>
              <DetailTerm
                label="Last observed"
                help="When StatusDeck most recently saw this incident in provider data."
              />
              <DetailDate value={incident.last_observed_at} />
            </div>
          </dl>
        </section>
      </div>
      <section className="card incident-timeline">
        <div className="section-heading">
          <div>
            <h2>Provider updates</h2>
          </div>
          <span className="badge neutral timeline-update-count">
            {updates.length} update{updates.length === 1 ? "" : "s"}
            {repeatedUpdates > 0 ? ` · ${repeatedUpdates} collapsed` : ""}
          </span>
        </div>
        {updateGroups.length ? (
          updateGroups.map((group, index) => {
            const update = group.updates[group.updates.length - 1];
            const latest = index === 0;
            return (
              <article
                className={`timeline-item${latest ? " latest" : ""}`}
                key={update.id}
              >
                <div className="timeline-marker" aria-hidden="true" />
                <div className="timeline-item-heading">
                  <h3>
                    {incidentUpdateStatus(
                      update.status,
                      providers.length === 1 ? providers[0].slug : undefined,
                    )}
                    {latest && <span className="latest-label">Latest</span>}
                  </h3>
                  {wasEdited(update) ? (
                    <span className="update-edited-time">
                      Edited <RelativeDateTime value={update.updated_at} />
                    </span>
                  ) : (
                    <RelativeDateTime
                      value={update.display_at ?? update.created_at}
                      fallback="Observed by StatusDeck"
                    />
                  )}
                </div>
                <ProviderContent value={update.body} />
                {update.scopes.length > 0 && (
                  <p className="muted">
                    Coverage reported for this update:{" "}
                    {update.scopes
                      .map(({ name, original_status }) =>
                        original_status
                          ? `${name} (${humanizeIdentifier(original_status)})`
                          : name,
                      )
                      .join(", ")}
                  </p>
                )}
                {update.synthesized && (
                  <p className="muted">Synthesized resolution</p>
                )}
                {group.updates.length > 1 && (
                  <details className="repeated-updates">
                    <summary>Repeated {group.updates.length} times</summary>
                    <ul>
                      {group.updates.map((repeatedUpdate) => (
                        <li key={repeatedUpdate.id}>
                          <RelativeDateTime
                            value={
                              repeatedUpdate.display_at ??
                              repeatedUpdate.created_at
                            }
                            fallback="Observed by StatusDeck"
                          />
                        </li>
                      ))}
                    </ul>
                  </details>
                )}
              </article>
            );
          })
        ) : (
          <EmptyState
            title="No provider updates"
            description="The provider has not published any timeline updates for this event."
            inline
          />
        )}
      </section>
      <IncidentComments key={incident.id} incidentId={incident.id} />
    </>
  );
}

function DetailTerm({ label, help }: { label: string; help: string }) {
  const tooltipId = useId();
  return (
    <dt className="incident-detail-term">
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
    </dt>
  );
}

function DetailDate({ value }: { value: string | null }) {
  return <dd>{value ? <RelativeDateTime value={value} /> : "Not reported"}</dd>;
}
