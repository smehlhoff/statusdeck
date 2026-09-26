import { useQuery } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { Dashboard, DashboardProvider } from "../../api/types";
import { SourceButton } from "../../components/SourceButton";
import { StatusBadge } from "../../components/StatusBadge";
import { AutoRefreshControl } from "../../components/AutoRefreshControl";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { useRefreshInterval } from "../../hooks/useRefreshInterval";
import { usePinnedProviders } from "../../hooks/usePinnedProviders";
import { humanizeIdentifier } from "../../utils/display";

const SUMMARY_ITEMS = [
  {
    key: "operational",
    label: "Operational",
    title: "Providers currently reporting normal operation.",
  },
  {
    key: "major_outage",
    label: "Major outage",
    title: "Providers currently reporting a major outage.",
  },
  {
    key: "partial_outage",
    label: "Partial outage",
    title: "Providers currently reporting a partial outage.",
  },
  {
    key: "degraded",
    label: "Degraded",
    title: "Providers currently reporting degraded provider.",
  },
  {
    key: "stale",
    label: "Stale monitoring",
    title: "Providers whose monitoring data needs attention.",
  },
] as const;

const PAGE_SIZE = 20;
const WALLBOARD_ROTATION_MS = 15_000;
const WALLBOARD_TWO_ROW_MIN_HEIGHT = 850;

type SummaryKey = (typeof SUMMARY_ITEMS)[number]["key"];
type ViewMode = "stacked" | "cards";

interface WallboardLayout {
  columns: number;
  rows: number;
}

function wallboardLayout(): WallboardLayout {
  if (typeof window === "undefined") return { columns: 4, rows: 2 };
  const { innerHeight: height, innerWidth: width } = window;
  let columns = 1;
  if (width >= 2_500) columns = 6;
  else if (width >= 1_750) columns = 5;
  else if (width >= 1_350) columns = 4;
  else if (width >= 1_000) columns = 3;
  else if (width >= 680) columns = 2;

  let rows = 1;
  if (height >= 1_700) rows = 4;
  else if (height >= 1_200) rows = 3;
  else if (height >= WALLBOARD_TWO_ROW_MIN_HEIGHT) rows = 2;
  return { columns, rows };
}

function wallboardPriority(provider: DashboardProvider): number {
  if (provider.status === "major_outage") return 0;
  if (provider.status === "partial_outage") return 1;
  if (provider.status === "degraded") return 2;
  if (!["fresh", "delayed"].includes(provider.freshness)) return 3;
  if (provider.status !== "operational") return 4;
  return 5;
}

function prioritizeProviders(providers: DashboardProvider[]) {
  return [...providers].sort(
    (left, right) =>
      wallboardPriority(left) - wallboardPriority(right) ||
      right.active_incidents - left.active_incidents ||
      left.name.localeCompare(right.name),
  );
}

function matchesSummary(provider: DashboardProvider, summary: SummaryKey) {
  if (summary === "stale") {
    return !["fresh", "delayed"].includes(provider.freshness);
  }
  return provider.status === summary;
}

function providersForSummary(
  providers: DashboardProvider[],
  summary: SummaryKey | null,
) {
  if (!summary) return providers;
  return providers.filter((provider) => matchesSummary(provider, summary));
}

export function Dashboard({ userId }: { userId: string }) {
  const [searchParams] = useSearchParams();
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const [viewMode, setViewMode] = useState<ViewMode>("stacked");
  const [wallboardPage, setWallboardPage] = useState(0);
  const [rotationPaused, setRotationPaused] = useState(false);
  const [layout, setLayout] = useState(wallboardLayout);
  const loadMoreRef = useRef<HTMLDivElement>(null);
  const { refreshInterval, cycleRefreshInterval } = useRefreshInterval();
  const { pinnedIds, togglePin, storageError } = usePinnedProviders(userId);
  const requestedSummary = searchParams.get("summary");
  const selectedSummary = SUMMARY_ITEMS.some(
    (item) => item.key === requestedSummary,
  )
    ? (requestedSummary as SummaryKey)
    : null;
  const query = useQuery({
    queryKey: queryKeys.dashboard,
    queryFn: ({ signal }) => api<Dashboard>("/api/v1/dashboard", { signal }),
    refetchInterval: refreshInterval || false,
    refetchIntervalInBackground: false,
  });

  const visibleProviders = query.data
    ? providersForSummary(query.data.providers, selectedSummary)
    : [];
  const pinnedProviders = visibleProviders
    .filter((provider) => pinnedIds.has(provider.id))
    .sort((left, right) => left.name.localeCompare(right.name));
  const stackedProviders = [
    ...pinnedProviders,
    ...visibleProviders.filter((provider) => !pinnedIds.has(provider.id)),
  ];
  const wallboardPageSize = layout.columns * layout.rows;
  // Reserve a rotating slot when all providers cannot fit on one screen.
  const fixedPins = pinnedProviders.slice(
    0,
    Math.max(
      wallboardPageSize - (visibleProviders.length > wallboardPageSize ? 1 : 0),
      0,
    ),
  );
  const fixedIds = new Set(fixedPins.map((provider) => provider.id));
  const wallboardProviders = prioritizeProviders(
    visibleProviders.filter((provider) => !fixedIds.has(provider.id)),
  ).sort(
    (left, right) =>
      Number(pinnedIds.has(right.id)) - Number(pinnedIds.has(left.id)),
  );
  const rotatingPageSize = Math.max(wallboardPageSize - fixedPins.length, 1);
  const wallboardPageCount = Math.max(
    Math.ceil(wallboardProviders.length / rotatingPageSize),
    1,
  );
  const currentWallboardPage = Math.min(wallboardPage, wallboardPageCount - 1);
  const wallboardPageStart = currentWallboardPage * rotatingPageSize;
  const displayedProviders =
    viewMode === "cards"
      ? [
          ...fixedPins,
          ...wallboardProviders.slice(
            wallboardPageStart,
            wallboardPageStart + rotatingPageSize,
          ),
        ]
      : stackedProviders.slice(0, visibleCount);
  const hasNextPage =
    viewMode === "stacked" &&
    displayedProviders.length < visibleProviders.length;

  useEffect(() => {
    setVisibleCount(PAGE_SIZE);
  }, [selectedSummary]);

  useEffect(() => {
    if (viewMode !== "cards") return;
    document.body.classList.add("wallboard-mode");
    const previousTitle = document.title;
    document.title = "StatusDeck · Provider health wallboard";
    const updateLayout = () => setLayout(wallboardLayout());
    const exitWallboard = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !document.querySelector("dialog[open]"))
        setViewMode("stacked");
    };
    updateLayout();
    window.addEventListener("resize", updateLayout);
    window.addEventListener("keydown", exitWallboard);
    return () => {
      document.body.classList.remove("wallboard-mode");
      document.title = previousTitle;
      window.removeEventListener("resize", updateLayout);
      window.removeEventListener("keydown", exitWallboard);
    };
  }, [viewMode]);

  useEffect(() => {
    setWallboardPage((current) =>
      Math.min(current, Math.max(wallboardPageCount - 1, 0)),
    );
  }, [selectedSummary, wallboardPageCount]);

  useEffect(() => {
    if (viewMode !== "cards" || rotationPaused || wallboardPageCount <= 1) {
      return;
    }
    const rotation = window.setInterval(() => {
      setWallboardPage((current) => (current + 1) % wallboardPageCount);
    }, WALLBOARD_ROTATION_MS);
    return () => window.clearInterval(rotation);
  }, [rotationPaused, viewMode, wallboardPageCount]);

  useEffect(() => {
    const target = loadMoreRef.current;
    if (!target || !hasNextPage) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisibleCount((current) =>
            Math.min(current + PAGE_SIZE, visibleProviders.length),
          );
        }
      },
      { rootMargin: "240px" },
    );
    observer.observe(target);
    return () => observer.disconnect();
  }, [hasNextPage, visibleProviders.length]);

  if (query.isLoading)
    return <LoadingSkeleton label="Loading overview" rows={4} />;
  if (!query.data)
    return (
      <EmptyState
        title="Overview unavailable"
        description="The dashboard could not be loaded. Check System or try again."
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
  const data = query.data;
  const wallboardStyles =
    viewMode === "cards"
      ? ({
          "--wallboard-columns": layout.columns,
          "--wallboard-rows": layout.rows,
        } as CSSProperties)
      : undefined;

  function toggleViewMode() {
    if (viewMode === "cards") {
      setViewMode("stacked");
      return;
    }
    setWallboardPage(0);
    setRotationPaused(false);
    setViewMode("cards");
  }

  return (
    <div
      className={viewMode === "cards" ? "dashboard-wallboard" : undefined}
      style={wallboardStyles}
    >
      <div className="page-heading dashboard-heading">
        <div>
          <h1>Overview</h1>
          <p className="muted">
            {viewMode === "cards"
              ? `${visibleProviders.length} monitored providers · ${fixedPins.length} pinned in place · other providers rotate by priority`
              : "Provider-reported health and monitoring freshness are shown separately."}
          </p>
        </div>
        <div className="heading-actions">
          {viewMode === "cards" && (
            <button
              className="button ghost"
              type="button"
              onClick={() =>
                window.dispatchEvent(new Event("statusdeck:open-search"))
              }
            >
              Search
            </button>
          )}
          <button
            className="button ghost dashboard-view-toggle"
            type="button"
            aria-label={
              viewMode === "cards" ? "Exit wall mode" : "Enter wall mode"
            }
            aria-pressed={viewMode === "cards"}
            onClick={toggleViewMode}
          >
            {viewMode === "cards" ? "Exit wall mode" : "Wall mode"}
          </button>
          <AutoRefreshControl
            interval={refreshInterval}
            onChange={cycleRefreshInterval}
          />
        </div>
      </div>
      {query.isError && (
        <p className="alert error" role="alert">
          Refresh failed. Showing the last dashboard update.
        </p>
      )}
      {storageError && (
        <p className="alert error" role="status">
          Pins work for this visit, but could not be saved in this browser.
        </p>
      )}
      {viewMode === "cards" && pinnedProviders.length > fixedPins.length && (
        <p className="wallboard-pin-notice" role="status">
          {pinnedProviders.length - fixedPins.length} additional pinned
          provider(s) rotate because this screen has no more fixed slots. Use a
          larger window or unpin providers to keep them in place.
        </p>
      )}
      <section
        className={`summary-grid dashboard-summary-grid${viewMode === "cards" ? " wallboard-summary-grid" : ""}`}
        aria-label="Status summary"
      >
        {SUMMARY_ITEMS.map((item) => {
          const count = data.counts[item.key] ?? 0;
          return (
            <Link
              className={`summary-card summary-link card${count > 0 ? ` summary-${item.key}` : ""}${selectedSummary === item.key ? " active" : ""}`}
              key={item.key}
              title={item.title}
              aria-current={selectedSummary === item.key ? "true" : undefined}
              to={selectedSummary === item.key ? "/" : `/?summary=${item.key}`}
            >
              <strong>{count}</strong>
              <span>{item.label}</span>
            </Link>
          );
        })}
      </section>
      {(viewMode === "stacked" || selectedSummary) && (
        <div
          className={`dashboard-results-summary${viewMode === "cards" ? " wallboard-results-summary" : ""}`}
          aria-live="polite"
        >
          {selectedSummary && (
            <div className="active-filters">
              <span className="muted">
                Showing {filterOptionLabel(selectedSummary)} providers
              </span>
              <Link className="clear-tag-filters" to="/">
                Clear filter
              </Link>
            </div>
          )}
          {viewMode === "stacked" && (
            <span className="muted dashboard-result-count">
              {displayedProviders.length}
              {hasNextPage ? "+" : ""} provider
              {displayedProviders.length === 1 ? "" : "s"} loaded
            </span>
          )}
        </div>
      )}
      {visibleProviders.length === 0 ? (
        <EmptyState
          title={
            selectedSummary ? "No matching providers" : "No monitored providers"
          }
          description={
            selectedSummary
              ? "Choose another status summary or clear the current filter."
              : "Subscribe to providers to begin monitoring their status."
          }
          action={
            <Link
              className="button ghost"
              to={selectedSummary ? "/" : "/catalog"}
            >
              {selectedSummary ? "Clear filter" : "Browse providers"}
            </Link>
          }
        />
      ) : (
        <>
          <div
            className={`provider-list${viewMode === "cards" ? " dashboard-card-grid" : ""}`}
          >
            {displayedProviders.map((provider) => (
              <ProviderRow
                key={provider.id}
                provider={provider}
                viewMode={viewMode}
                pinned={pinnedIds.has(provider.id)}
                onTogglePin={() => togglePin(provider.id)}
              />
            ))}
          </div>
          {viewMode === "stacked" ? (
            <div className="infinite-scroll-sentinel" ref={loadMoreRef}>
              {hasNextPage ? "Scroll to load more providers" : null}
            </div>
          ) : (
            wallboardPageCount > 1 && (
              <nav
                className="wallboard-pagination"
                aria-label="Wallboard pages"
              >
                <button
                  className="button ghost compact"
                  type="button"
                  aria-label="Show first wallboard page"
                  disabled={wallboardPage === 0}
                  onClick={() => setWallboardPage(0)}
                >
                  First
                </button>
                <button
                  className="button ghost compact"
                  type="button"
                  aria-label="Show previous wallboard page"
                  onClick={() =>
                    setWallboardPage(
                      (current) =>
                        (current - 1 + wallboardPageCount) % wallboardPageCount,
                    )
                  }
                >
                  Previous
                </button>
                <span>
                  Screen {wallboardPage + 1} of {wallboardPageCount}
                </span>
                <button
                  className="button ghost compact"
                  type="button"
                  onClick={() => setRotationPaused((current) => !current)}
                >
                  {rotationPaused ? "Resume rotation" : "Pause rotation"}
                </button>
                <button
                  className="button ghost compact"
                  type="button"
                  aria-label="Show next wallboard page"
                  onClick={() =>
                    setWallboardPage(
                      (current) => (current + 1) % wallboardPageCount,
                    )
                  }
                >
                  Next
                </button>
                <button
                  className="button ghost compact"
                  type="button"
                  aria-label="Show last wallboard page"
                  disabled={wallboardPage === wallboardPageCount - 1}
                  onClick={() => setWallboardPage(wallboardPageCount - 1)}
                >
                  Last
                </button>
              </nav>
            )
          )}
        </>
      )}
    </div>
  );
}

function filterOptionLabel(value: string) {
  return humanizeIdentifier(value);
}

function ProviderRow({
  provider,
  viewMode,
  pinned,
  onTogglePin,
}: {
  provider: DashboardProvider;
  viewMode: ViewMode;
  pinned: boolean;
  onTogglePin: () => void;
}) {
  const attention =
    provider.freshness !== "fresh" && provider.freshness !== "delayed";
  const hasEvents =
    provider.active_incidents > 0 ||
    provider.open_maintenance > 0 ||
    provider.upcoming_maintenance > 0;

  return (
    <article
      className={`card provider-row${viewMode === "cards" ? " dashboard-provider-card" : ""}`}
      data-status={provider.status}
      data-monitoring-attention={attention ? "true" : undefined}
      data-pinned={pinned ? "true" : undefined}
    >
      <div className="provider-main">
        <div className="provider-title">
          <h2>
            <Link to={`/catalog/${provider.id}`}>{provider.name}</Link>
            <button
              className="provider-pin"
              type="button"
              aria-label={`${pinned ? "Unpin" : "Pin"} ${provider.name}`}
              aria-pressed={pinned}
              title={pinned ? "Unpin provider" : "Pin provider"}
              onClick={onTogglePin}
            >
              <svg
                viewBox="0 0 24 24"
                width="14"
                height="14"
                aria-hidden="true"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.7"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <g transform="rotate(45 12 12)">
                  <path
                    d="M8 3h8l-1 6 3 3v2H6v-2l3-3-1-6Z"
                    fill={pinned ? "currentColor" : "none"}
                    fillOpacity="0.16"
                  />
                  <path d="M12 14v7" />
                </g>
              </svg>
            </button>
          </h2>
          <StatusBadge value={provider.status} />
          {attention && (
            <StatusBadge
              value={provider.freshness}
              tone="warning"
              label={`monitoring ${humanizeIdentifier(provider.freshness)}`}
            />
          )}
        </div>
        {viewMode === "cards" ? (
          <dl className="dashboard-card-details">
            <div>
              <dt>Coverage</dt>
              <dd>{humanizeIdentifier(provider.status)}</dd>
            </div>
            <div>
              <dt>Provider-wide</dt>
              <dd>{humanizeIdentifier(provider.provider_status)}</dd>
            </div>
            <div>
              <dt>Last checked</dt>
              <dd>
                <RelativeDateTime
                  value={provider.last_success_at}
                  fallback="never"
                />
              </dd>
            </div>
          </dl>
        ) : (
          <p className="muted">
            Monitored coverage: {humanizeIdentifier(provider.status)} ·
            Provider-wide: {humanizeIdentifier(provider.provider_status)} · Last
            checked{" "}
            <RelativeDateTime
              value={provider.last_success_at}
              fallback="never"
            />
          </p>
        )}
        {provider.outside_scope_incidents > 0 && (
          <p className="status-context">
            {viewMode === "cards" ? (
              <>
                {provider.outside_scope_incidents} open outside monitored
                coverage
              </>
            ) : (
              <>
                {provider.outside_scope_incidents} open provider{" "}
                {provider.outside_scope_incidents === 1
                  ? "incident exists"
                  : "incidents exist"}{" "}
                outside your configured coverage.{" "}
                <Link to={`/incidents?provider_id=${provider.id}&scope=all`}>
                  View all provider incidents
                </Link>
              </>
            )}
          </p>
        )}
        {provider.affected_components.length > 0 && (
          <p
            className="provider-affected-components"
            title={
              viewMode === "cards"
                ? `Affected: ${provider.affected_components.join(", ")}`
                : undefined
            }
          >
            Affected:{" "}
            {viewMode === "cards"
              ? `${provider.affected_components.length} component${provider.affected_components.length === 1 ? "" : "s"}`
              : provider.affected_components.join(", ")}
          </p>
        )}
        {provider.status !== "operational" &&
          provider.active_incidents === 0 &&
          provider.open_maintenance === 0 && (
            <p className="status-context">
              Provider status only · no matching open incident published
            </p>
          )}
      </div>
      <div className="provider-events" aria-label="Open provider events">
        {hasEvents ? (
          <>
            <Link
              className="event-count"
              to={`/incidents?provider_id=${provider.id}&kind=incident&activity=active`}
            >
              <strong>{provider.active_incidents}</strong>
              <span>Incidents</span>
            </Link>
            <Link
              className="event-count"
              to={`/incidents?provider_id=${provider.id}&kind=maintenance&maintenance_window=active`}
            >
              <strong>{provider.open_maintenance}</strong>
              <span>Scheduled maintenance now</span>
            </Link>
            <Link
              className="event-count"
              to={`/incidents?provider_id=${provider.id}&kind=maintenance&maintenance_window=upcoming`}
            >
              <strong>{provider.upcoming_maintenance}</strong>
              <span>Upcoming</span>
            </Link>
          </>
        ) : (
          <span className="event-link placeholder">
            No current or upcoming events
          </span>
        )}
      </div>
      <SourceButton href={provider.official_url} className="provider-source" />
      {viewMode === "stacked" && (
        <div
          className="tag-list catalog-tags provider-tags"
          aria-label="Provider categories"
        >
          {provider.tags.map((tag) => (
            <span className="tag static" key={tag}>
              {tag}
            </span>
          ))}
        </div>
      )}
    </article>
  );
}
