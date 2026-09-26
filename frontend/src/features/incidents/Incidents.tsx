import { useProfile } from "../profile/profileContext";
import type { DisplayPreferences } from "../../api/types";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { CatalogProvider, IncidentPage } from "../../api/types";
import { SourceButton } from "../../components/SourceButton";
import { AutoRefreshControl } from "../../components/AutoRefreshControl";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { useRefreshInterval } from "../../hooks/useRefreshInterval";
import {
  dateTimeInputIso,
  dateTimeInputValue,
  formatDateTime,
  humanizeIdentifier,
  maintenanceTiming,
} from "../../utils/display";
import { selectSearchParams } from "../../utils/searchParams";
import { BookmarkButton } from "./BookmarkButton";
import { CopyIncidentLinkButton } from "./CopyIncidentLinkButton";

const ADVANCED_FILTER_NAMES = [
  "latest_phase",
  "affected_status",
  "updated_within",
  "from",
  "to",
] as const;
const ADVANCED_FILTERS_STORAGE_KEY =
  "statusdeck-incidents-advanced-filters-open";

const FILTER_NAMES = [
  "q",
  "provider_id",
  "provider_tag",
  "kind",
  "scope",
  "maintenance_window",
  "upcoming_within",
  "severity",
  "lifecycle",
  "latest_phase",
  "affected_status",
  "updated_within",
  "from",
  "to",
] as const;

const INCIDENT_FILTER_NAMES = [
  ...FILTER_NAMES,
  "activity",
  "component_id",
  "provider_ids",
] as const;

const INCIDENT_API_FILTER_NAMES = INCIDENT_FILTER_NAMES.filter(
  (name) => name !== "provider_tag",
);

function filterValue(
  name: (typeof FILTER_NAMES)[number],
  value: string,
  preferences: DisplayPreferences,
) {
  return name === "from" || name === "to"
    ? dateTimeInputIso(value, preferences)
    : value;
}

function filterOptionLabel(value: string) {
  const label = humanizeIdentifier(value);
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function storedAdvancedFiltersOpen(): boolean {
  try {
    return localStorage.getItem(ADVANCED_FILTERS_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function activeFilterLabel(
  name: string,
  value: string,
  providers: CatalogProvider[] | undefined,
  preferences: DisplayPreferences,
) {
  const labels: Record<string, string> = {
    q: "Search",
    provider_id: "Provider",
    component_id: "Component",
    provider_ids: "Providers",
    provider_tag: "Category",
    kind: "Kind",
    scope: "Coverage",
    maintenance_window: "Maintenance",
    upcoming_within: "Horizon",
    severity: "Severity",
    lifecycle: "Lifecycle",
    activity: "Lifecycle",
    latest_phase: "Latest phase",
    affected_status: "Component status",
    updated_within: "Updated",
    from: "From",
    to: "To",
  };
  let displayValue = filterOptionLabel(value);
  if (name === "provider_id") {
    displayValue =
      providers?.find((provider) => provider.id === value)?.name ?? value;
  } else if (name === "scope" && value === "all") {
    displayValue = "All provider events";
  } else if (name === "activity" && value === "active") {
    displayValue = "Active";
  } else if (name === "from" || name === "to") {
    displayValue = formatDateTime(value, preferences);
  }
  return `${labels[name] ?? filterOptionLabel(name)}: ${displayValue}`;
}

export function Incidents() {
  const { preferences } = useProfile();
  const [urlSearchParams, setSearchParams] = useSearchParams();
  const searchParams = selectSearchParams(
    urlSearchParams,
    INCIDENT_FILTER_NAMES,
  );
  const hasAdvancedFilters = ADVANCED_FILTER_NAMES.some((name) =>
    searchParams.has(name),
  );
  const [advancedFiltersOpen, setAdvancedFiltersOpen] = useState(
    () => hasAdvancedFilters || storedAdvancedFiltersOpen(),
  );
  const { refreshInterval, cycleRefreshInterval } = useRefreshInterval();
  const filterQuery = searchParams.toString();
  const initialProviderId = searchParams.get("provider_id") ?? "";
  const initialUpcomingWithin = searchParams.get("upcoming_within") ?? "";
  const [providerId, setProviderId] = useState(initialProviderId);
  const [kind, setKind] = useState(searchParams.get("kind") ?? "");
  const [maintenanceWindow, setMaintenanceWindow] = useState(
    searchParams.get("maintenance_window") ??
      (initialUpcomingWithin ? "upcoming" : ""),
  );
  const [upcomingWithin, setUpcomingWithin] = useState(initialUpcomingWithin);
  const catalog = useQuery({
    queryKey: queryKeys.catalog,
    queryFn: ({ signal }) =>
      api<CatalogProvider[]>("/api/v1/catalog/providers", { signal }),
  });
  const providerTagOptions = Array.from(
    new Set(catalog.data?.flatMap((provider) => provider.tags) ?? []),
  ).sort((left, right) => left.localeCompare(right));
  const query = useInfiniteQuery({
    queryKey: queryKeys.incidentList(filterQuery),
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) => {
      const providerTag = searchParams.get("provider_tag");
      const params = selectSearchParams(
        searchParams,
        INCIDENT_API_FILTER_NAMES,
      );
      params.set("limit", "20");
      if (providerTag) {
        const providerIds =
          catalog.data
            ?.filter((provider) => provider.tags.includes(providerTag))
            .map((provider) => provider.id) ?? [];
        if (providerIds.length === 0) {
          return { items: [], next_cursor: null };
        }
        params.set("provider_ids", providerIds.join(","));
      }
      if (pageParam) params.set("cursor", pageParam);
      return api<IncidentPage>(`/api/v1/incidents?${params.toString()}`, {
        signal,
      });
    },
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: !searchParams.has("provider_tag") || Boolean(catalog.data),
    refetchInterval: (currentQuery) => {
      if (refreshInterval === 0) return false;
      const data = currentQuery.state.data;
      return data && data.pages.length > 1 ? false : refreshInterval;
    },
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
  });
  const {
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isFetchNextPageError,
  } = query;

  const incidents = query.data?.pages.flatMap((page) => page.items) ?? [];
  const activeFilters = Array.from(searchParams.entries()).filter(
    ([name, value]) => value && !(name === "scope" && value === "monitored"),
  );
  const loadMoreRef = useRef<HTMLDivElement>(null);
  let body: ReactNode;

  useEffect(() => {
    const current = new URLSearchParams(filterQuery);
    const horizon = current.get("upcoming_within") ?? "";
    setProviderId(current.get("provider_id") ?? "");
    setKind(current.get("kind") ?? "");
    setMaintenanceWindow(
      current.get("maintenance_window") ?? (horizon ? "upcoming" : ""),
    );
    setUpcomingWithin(horizon);
  }, [filterQuery]);

  useEffect(() => {
    if (hasAdvancedFilters) setAdvancedFiltersOpen(true);
  }, [hasAdvancedFilters]);

  useEffect(() => {
    const target = loadMoreRef.current;
    if (!target || !hasNextPage || isFetchNextPageError) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting && !isFetchingNextPage) {
          void fetchNextPage();
        }
      },
      { rootMargin: "240px" },
    );
    observer.observe(target);
    return () => observer.disconnect();
  }, [fetchNextPage, hasNextPage, isFetchingNextPage, isFetchNextPageError]);

  function applyFilters(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const next = new URLSearchParams();
    // Drill-down links carry scope filters without corresponding form fields.
    for (const name of ["component_id", "provider_ids"]) {
      const value = searchParams.get(name);
      if (value) next.set(name, value);
    }
    for (const name of FILTER_NAMES) {
      const value = form.get(name)?.toString();
      if (value) {
        if (name === "lifecycle" && value === "active") {
          next.set("activity", "active");
          continue;
        }
        next.set(name, filterValue(name, value, preferences));
      }
    }
    if (next.has("maintenance_window")) {
      next.set("kind", "maintenance");
    }
    if (next.has("upcoming_within")) {
      next.set("kind", "maintenance");
      next.set("maintenance_window", "upcoming");
    } else if (next.get("maintenance_window") !== "upcoming") {
      next.delete("upcoming_within");
    }
    setSearchParams(next);
  }

  function removeFilter(name: string) {
    const next = new URLSearchParams(searchParams);
    next.delete(name);
    if (name === "maintenance_window") next.delete("upcoming_within");
    if (name === "kind") {
      next.delete("maintenance_window");
      next.delete("upcoming_within");
    }
    setSearchParams(next);
  }

  function filterByProvider(providerId: string) {
    const next = new URLSearchParams(searchParams);
    next.set("provider_id", providerId);
    next.delete("provider_tag");
    next.delete("provider_ids");
    next.delete("component_id");
    setSearchParams(next);
  }

  const needsCatalog = searchParams.has("provider_tag");
  if (query.isLoading || (needsCatalog && catalog.isLoading)) {
    body = <LoadingSkeleton label="Loading incidents" rows={4} />;
  } else if (
    (query.isError && !query.data) ||
    (needsCatalog && catalog.isError && !catalog.data)
  ) {
    body = (
      <EmptyState
        title="Incident history unavailable"
        description="Incident history could not be loaded. Check the connection and try again."
        error
        action={
          <button
            className="button ghost"
            type="button"
            onClick={() => {
              if (needsCatalog && !catalog.isSuccess) void catalog.refetch();
              else void query.refetch();
            }}
          >
            Retry
          </button>
        }
      />
    );
  } else if (incidents.length > 0) {
    body = (
      <>
        <div className="timeline">
          {incidents.map((incident) => (
            <article
              className={`card incident-row ${incident.severity}`}
              key={incident.id}
            >
              <div className="incident-row-main">
                <div className="badge-row">
                  {incident.providers.map((provider) => (
                    <button
                      className="badge provider-badge"
                      type="button"
                      key={provider.id}
                      aria-label={`Filter incidents by ${provider.name}`}
                      aria-pressed={
                        searchParams.get("provider_id") === provider.id
                      }
                      onClick={() => filterByProvider(provider.id)}
                    >
                      {provider.name}
                    </button>
                  ))}
                  <span className={"badge " + incident.severity}>
                    {incident.severity}
                  </span>
                  <span className="badge neutral">
                    {humanizeIdentifier(incident.lifecycle)}
                  </span>
                  <span className="badge neutral">{incident.kind}</span>
                  {incident.kind === "maintenance" &&
                    incident.lifecycle !== "resolved" && (
                      <span className="badge neutral">
                        {maintenanceTiming(
                          incident.planned_start_at,
                          incident.planned_end_at,
                        )}
                      </span>
                    )}
                  {!incident.within_provider_scope && (
                    <span className="badge warning">
                      Outside provider scope
                    </span>
                  )}
                </div>
                <h2>
                  <Link
                    to={`/incidents/${incident.id}`}
                    state={{ incidentSearch: filterQuery }}
                  >
                    {incident.title}
                  </Link>
                </h2>
                <p className="muted incident-row-footer">
                  First reported{" "}
                  <RelativeDateTime
                    value={incident.provider_created_at}
                    fallback="Not provided"
                  />
                  {" · Latest update "}
                  <RelativeDateTime
                    value={incident.provider_updated_at}
                    fallback="Not provided"
                  />
                </p>
              </div>
              <div className="incident-actions">
                <BookmarkButton
                  incidentId={incident.id}
                  bookmarked={incident.bookmarked}
                />
                <CopyIncidentLinkButton incidentId={incident.id} />
                {incident.official_url && (
                  <SourceButton href={incident.official_url} />
                )}
              </div>
            </article>
          ))}
        </div>
        <div className="infinite-scroll-sentinel" ref={loadMoreRef}>
          {query.isFetchingNextPage ? "Loading more incidents…" : null}
        </div>
      </>
    );
  } else {
    body = (
      <EmptyState
        title="No matching incidents"
        description="No provider events match the current filters. Clear them to view all available incidents and maintenance."
        action={
          activeFilters.length > 0 ? (
            <button
              className="button ghost"
              type="button"
              onClick={() => setSearchParams({})}
            >
              Clear filters
            </button>
          ) : undefined
        }
      />
    );
  }

  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Incidents</h1>
          <p className="muted">
            Provider updates are retained as a newest-first, auditable record.
          </p>
        </div>
        <AutoRefreshControl
          interval={refreshInterval}
          onChange={cycleRefreshInterval}
        />
      </div>
      <form
        className="card incident-filters"
        key={filterQuery}
        onSubmit={applyFilters}
      >
        <div className="incident-filter-header">
          <div>
            <h2>Find incidents</h2>
          </div>
          <div className="filter-actions">
            <button className="button primary">Apply filters</button>
            <button
              className="button ghost"
              type="button"
              onClick={(event) => {
                event.currentTarget.form?.reset();
                setProviderId("");
                setKind("");
                setMaintenanceWindow("");
                setUpcomingWithin("");
                setSearchParams({});
              }}
            >
              Clear
            </button>
          </div>
        </div>
        <div
          className="incident-filter-section"
          role="group"
          aria-labelledby="provider-filter-heading"
        >
          <h3 id="provider-filter-heading">Provider and search</h3>
          <div className="filter-grid">
            <label>
              Search
              <input
                name="q"
                maxLength={500}
                defaultValue={searchParams.get("q") ?? ""}
                placeholder="Title or provider reference"
              />
            </label>
            <label>
              Provider
              <select
                name="provider_id"
                value={providerId}
                onChange={(event) => setProviderId(event.target.value)}
              >
                <option value="">All providers</option>
                {catalog.data?.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Provider category
              <select
                name="provider_tag"
                defaultValue={searchParams.get("provider_tag") ?? ""}
              >
                <option value="">All categories</option>
                {providerTagOptions.map((tag) => (
                  <option key={tag}>{tag}</option>
                ))}
              </select>
            </label>
          </div>
        </div>
        <div
          className="incident-filter-section"
          role="group"
          aria-labelledby="event-filter-heading"
        >
          <h3 id="event-filter-heading">Event details</h3>
          <div className="filter-grid">
            <label>
              Kind
              <select
                name="kind"
                value={kind}
                onChange={(event) => {
                  const value = event.target.value;
                  setKind(value);
                  if (value !== "maintenance") {
                    setMaintenanceWindow("");
                    setUpcomingWithin("");
                  }
                }}
              >
                <option value="">Incidents and maintenance</option>
                <option value="incident">Incident</option>
                <option value="maintenance">Maintenance</option>
              </select>
            </label>
            <label>
              Coverage
              <select
                name="scope"
                defaultValue={searchParams.get("scope") ?? "monitored"}
              >
                <option value="monitored">My monitored coverage</option>
                <option value="all">All provider events</option>
              </select>
            </label>
            <label>
              Maintenance timing
              <select
                name="maintenance_window"
                value={maintenanceWindow}
                onChange={(event) => {
                  const value = event.target.value;
                  setMaintenanceWindow(value);
                  if (value) setKind("maintenance");
                  if (value !== "upcoming") setUpcomingWithin("");
                }}
              >
                <option value="">Any maintenance timing</option>
                <option value="active">Happening now</option>
                <option value="upcoming">Upcoming</option>
              </select>
            </label>
            <label>
              Upcoming horizon
              <select
                name="upcoming_within"
                value={upcomingWithin}
                onChange={(event) => {
                  const value = event.target.value;
                  setUpcomingWithin(value);
                  if (value) {
                    setKind("maintenance");
                    setMaintenanceWindow("upcoming");
                  }
                }}
              >
                <option value="">Any future date</option>
                <option value="7d">Next 7 days</option>
                <option value="30d">Next 30 days</option>
              </select>
            </label>
            <label>
              Severity
              <select
                name="severity"
                defaultValue={searchParams.get("severity") ?? ""}
              >
                <option value="">All severities</option>
                {["info", "minor", "major", "critical"].map((value) => (
                  <option key={value} value={value}>
                    {filterOptionLabel(value)}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Lifecycle
              <select
                name="lifecycle"
                defaultValue={
                  searchParams.get("activity") === "active"
                    ? "active"
                    : (searchParams.get("lifecycle") ?? "")
                }
              >
                <option value="">All states</option>
                <option value="active">
                  Active (open or resolution pending)
                </option>
                <option value="open">Open</option>
                <option value="resolution_pending">Resolution pending</option>
                <option value="resolved">Resolved</option>
              </select>
            </label>
          </div>
        </div>
        <details
          className="incident-filter-details"
          open={advancedFiltersOpen}
          onToggle={(event) => {
            const open = event.currentTarget.open;
            setAdvancedFiltersOpen(open);
            try {
              localStorage.setItem(ADVANCED_FILTERS_STORAGE_KEY, String(open));
            } catch {
              // The preference is optional when browser storage is unavailable.
            }
          }}
        >
          <summary>Advanced filters</summary>
          <div className="filter-grid">
            <label>
              Latest update phase
              <select
                name="latest_phase"
                defaultValue={searchParams.get("latest_phase") ?? ""}
              >
                <option value="">All phases</option>
                {["investigating", "identified", "monitoring", "resolved"].map(
                  (value) => (
                    <option key={value} value={value}>
                      {filterOptionLabel(value)}
                    </option>
                  ),
                )}
              </select>
            </label>
            <label>
              Affected component status
              <select
                name="affected_status"
                defaultValue={searchParams.get("affected_status") ?? ""}
              >
                <option value="">All component states</option>
                {[
                  "operational",
                  "maintenance",
                  "degraded",
                  "partial_outage",
                  "major_outage",
                  "unknown",
                ].map((value) => (
                  <option key={value} value={value}>
                    {filterOptionLabel(value)}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Recently updated
              <select
                name="updated_within"
                defaultValue={searchParams.get("updated_within") ?? ""}
              >
                <option value="">Any time</option>
                <option value="24h">Last 24 hours</option>
                <option value="7d">Last 7 days</option>
                <option value="30d">Last 30 days</option>
              </select>
            </label>
            <label>
              From
              <input
                name="from"
                type="datetime-local"
                defaultValue={dateTimeInputValue(
                  searchParams.get("from"),
                  preferences,
                )}
              />
            </label>
            <label>
              To
              <input
                name="to"
                type="datetime-local"
                defaultValue={dateTimeInputValue(
                  searchParams.get("to"),
                  preferences,
                )}
              />
            </label>
          </div>
        </details>
      </form>
      <div className="incident-results-summary" aria-live="polite">
        {activeFilters.length > 0 && (
          <div className="active-filters" aria-label="Active incident filters">
            {activeFilters.map(([name, value]) => (
              <button
                className="active-filter"
                key={name}
                type="button"
                onClick={() => removeFilter(name)}
              >
                {activeFilterLabel(name, value, catalog.data, preferences)}{" "}
                <span aria-hidden="true">×</span>
                <span className="sr-only">Remove filter</span>
              </button>
            ))}
            <button
              className="clear-tag-filters"
              type="button"
              onClick={() => setSearchParams({})}
            >
              Clear all
            </button>
          </div>
        )}
        <span className="muted incident-result-count">
          {query.isLoading
            ? "Loading results…"
            : `${incidents.length}${hasNextPage ? "+" : ""} event${incidents.length === 1 ? "" : "s"} loaded`}
        </span>
      </div>
      {needsCatalog && catalog.isError && catalog.data && (
        <p className="alert error" role="alert">
          Provider categories could not be refreshed. Using the last loaded
          catalog for this filter.{" "}
          <button
            className="button ghost"
            disabled={catalog.isFetching}
            onClick={() => void catalog.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      {query.isError && query.data && (
        <p className="alert error" role="alert">
          {isFetchNextPageError
            ? "More incidents could not be loaded. Showing the events already loaded."
            : "Refresh failed. Showing the last incident update."}{" "}
          <button
            className="button ghost"
            disabled={query.isFetching}
            onClick={() => {
              if (isFetchNextPageError) void fetchNextPage();
              else void query.refetch();
            }}
          >
            Retry
          </button>
        </p>
      )}
      {body}
    </>
  );
}
