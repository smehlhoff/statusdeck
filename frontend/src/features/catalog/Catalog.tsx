import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { api } from "../../api/client";
import { invalidateMonitoringState, queryKeys } from "../../api/queries";
import type { CatalogDetail, CatalogProvider, Monitor } from "../../api/types";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { useToast } from "../../components/toastContext";

interface BulkMonitorResult {
  succeeded: number;
  failed: number;
  firstError?: string;
}

const PAGE_SIZE = 20;

async function runBulkMonitorAction<T>(
  items: T[],
  operation: (item: T) => Promise<unknown>,
): Promise<BulkMonitorResult> {
  const result: BulkMonitorResult = { succeeded: 0, failed: 0 };
  for (const item of items) {
    try {
      await operation(item);
      result.succeeded += 1;
    } catch (error) {
      result.failed += 1;
      result.firstError ??=
        error instanceof Error ? error.message : "Request failed";
    }
  }
  return result;
}

function bulkResultMessage(action: string, result: BulkMonitorResult): string {
  const summary = `${action} ${result.succeeded} provider${result.succeeded === 1 ? "" : "s"}`;
  return result.failed === 0
    ? `${summary}.`
    : `${summary}; ${result.failed} failed. ${result.firstError ?? "Try the failed providers again."}`;
}

export function Catalog() {
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const [search, setSearch] = useState("");
  const [subscribedOnly, setSubscribedOnly] = useState(false);
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const loadMoreRef = useRef<HTMLDivElement>(null);
  const [bulkStatus, setBulkStatus] = useState<{
    message: string;
    failed: boolean;
  }>();
  const [confirmUnsubscribeAll, setConfirmUnsubscribeAll] = useState(false);

  const providers = useQuery({
    queryKey: queryKeys.catalog,
    queryFn: ({ signal }) =>
      api<CatalogProvider[]>("/api/v1/catalog/providers", { signal }),
  });

  const monitors = useQuery({
    queryKey: queryKeys.monitors,
    queryFn: ({ signal }) => api<Monitor[]>("/api/v1/monitors", { signal }),
  });

  const enabledProviderIds = new Set(
    monitors.data
      ?.filter((monitor) => monitor.enabled)
      .map((monitor) => monitor.provider_id) ?? [],
  );
  const unsubscribedProviders =
    providers.data?.filter(
      (provider) => !enabledProviderIds.has(provider.id),
    ) ?? [];
  const enabledMonitors =
    monitors.data?.filter((monitor) => monitor.enabled) ?? [];

  const subscribeAll = useMutation({
    mutationFn: () =>
      runBulkMonitorAction(unsubscribedProviders, (provider) =>
        api<Monitor>("/api/v1/monitors", {
          method: "POST",
          body: JSON.stringify({
            provider_id: provider.id,
            monitor_all_components: true,
            component_ids: [],
          }),
        }),
      ),
    onMutate: () => setBulkStatus(undefined),
    onSuccess: (result) => {
      notify(
        bulkResultMessage("Subscribed to", result),
        result.failed > 0 ? "error" : "success",
      );
      setBulkStatus({
        message: bulkResultMessage("Subscribed to", result),
        failed: result.failed > 0,
      });
    },
    onSettled: refreshAllMonitoringState,
  });

  const unsubscribeAll = useMutation({
    mutationFn: () =>
      runBulkMonitorAction(enabledMonitors, (monitor) =>
        api<void>(`/api/v1/monitors/${monitor.id}`, {
          method: "DELETE",
        }),
      ),
    onMutate: () => setBulkStatus(undefined),
    onSuccess: (result) => {
      notify(
        bulkResultMessage("Unsubscribed from", result),
        result.failed > 0 ? "error" : "success",
      );
      setBulkStatus({
        message: bulkResultMessage("Unsubscribed from", result),
        failed: result.failed > 0,
      });
      setConfirmUnsubscribeAll(false);
    },
    onSettled: refreshAllMonitoringState,
  });

  function refreshAllMonitoringState() {
    void invalidateMonitoringState(queryClient);
  }

  const normalizedSearch = search.toLowerCase();
  const filteredProviders =
    providers.data?.filter((provider) => {
      const searchable = [provider.name, provider.description, ...provider.tags]
        .join(" ")
        .toLowerCase();
      return (
        searchable.includes(normalizedSearch) &&
        (!subscribedOnly || enabledProviderIds.has(provider.id)) &&
        selectedTags.every((tag) => provider.tags.includes(tag))
      );
    }) ?? [];
  const displayedProviders = filteredProviders.slice(0, visibleCount);
  const hasNextPage = displayedProviders.length < filteredProviders.length;

  useEffect(() => {
    setVisibleCount(PAGE_SIZE);
  }, [normalizedSearch, selectedTags, subscribedOnly]);

  useEffect(() => {
    const target = loadMoreRef.current;
    if (!target || !hasNextPage) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisibleCount((current) =>
            Math.min(current + PAGE_SIZE, filteredProviders.length),
          );
        }
      },
      { rootMargin: "240px" },
    );
    observer.observe(target);
    return () => observer.disconnect();
  }, [
    filteredProviders.length,
    hasNextPage,
    monitors.isLoading,
    providers.isLoading,
  ]);

  let catalogContent: ReactNode;

  if (providers.isLoading || monitors.isLoading) {
    catalogContent = <LoadingSkeleton label="Loading providers" rows={4} />;
  } else if (
    (providers.isError && !providers.data) ||
    (monitors.isError && !monitors.data)
  ) {
    catalogContent = (
      <EmptyState
        title="Catalog unavailable"
        description="The provider catalog could not be loaded. Check the connection and try again."
        error
        action={
          <button
            className="button ghost"
            type="button"
            onClick={() => {
              void providers.refetch();
              void monitors.refetch();
            }}
          >
            Retry
          </button>
        }
      />
    );
  } else if (filteredProviders.length === 0) {
    catalogContent = (
      <EmptyState
        title={
          search || selectedTags.length || subscribedOnly
            ? "No matching providers"
            : "No providers available"
        }
        description={
          search || selectedTags.length || subscribedOnly
            ? "Try a different search or clear the selected filters."
            : "Providers will appear here when they are available in the catalog."
        }
        action={
          search || selectedTags.length || subscribedOnly ? (
            <button
              className="button ghost"
              type="button"
              onClick={() => {
                setSearch("");
                setSelectedTags([]);
                setSubscribedOnly(false);
              }}
            >
              Clear filters
            </button>
          ) : undefined
        }
      />
    );
  } else {
    catalogContent = (
      <>
        <div className="progressive-list-summary" aria-live="polite">
          <span className="muted">
            {displayedProviders.length}
            {hasNextPage ? "+" : ""} provider
            {displayedProviders.length === 1 ? "" : "s"} loaded
          </span>
        </div>
        <div className="catalog-grid">
          {displayedProviders.map((provider) => (
            <CatalogCard
              key={provider.id}
              provider={provider}
              monitor={monitors.data?.find(
                (monitor) => monitor.provider_id === provider.id,
              )}
              selectedTags={selectedTags}
              onTagSelect={(tag) =>
                setSelectedTags((current) =>
                  current.includes(tag)
                    ? current.filter((selected) => selected !== tag)
                    : [...current, tag],
                )
              }
            />
          ))}
        </div>
        <div className="infinite-scroll-sentinel" ref={loadMoreRef}>
          {hasNextPage ? "Scroll to load more providers" : null}
        </div>
      </>
    );
  }

  return (
    <>
      <div className="page-heading catalog-heading">
        <div>
          <h1>Providers</h1>
          <p className="muted">
            Subscribe to whole providers or choose only the components you rely
            on.
          </p>
        </div>
        <div className="heading-actions catalog-bulk-actions">
          <button
            className="button primary"
            disabled={
              providers.isLoading ||
              !monitors.data ||
              subscribeAll.isPending ||
              unsubscribeAll.isPending ||
              unsubscribedProviders.length === 0
            }
            onClick={() => subscribeAll.mutate()}
          >
            {subscribeAll.isPending ? "Subscribing…" : "Subscribe all"}
          </button>
          <button
            className="button danger"
            disabled={
              monitors.isLoading ||
              !monitors.data ||
              unsubscribeAll.isPending ||
              subscribeAll.isPending ||
              enabledMonitors.length === 0
            }
            onClick={() => setConfirmUnsubscribeAll(true)}
          >
            {unsubscribeAll.isPending ? "Unsubscribing…" : "Unsubscribe all"}
          </button>
        </div>
      </div>
      <div className="catalog-layout">
        <div className="catalog-toolbar">
          <label className="search">
            <span className="sr-only">Search providers</span>
            <input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search by provider name or category"
            />
          </label>
          <button
            className={`button ${subscribedOnly ? "primary" : "ghost"}`}
            type="button"
            aria-pressed={subscribedOnly}
            onClick={() => setSubscribedOnly((current) => !current)}
          >
            Subscribed only
          </button>
        </div>
        {bulkStatus && (
          <p
            className={`alert${bulkStatus.failed ? " error" : ""}`}
            role={bulkStatus.failed ? "alert" : "status"}
          >
            {bulkStatus.message}
          </p>
        )}
        {selectedTags.length > 0 && (
          <div className="active-filters" aria-label="Selected categories">
            <span className="muted">Matches all:</span>
            {selectedTags.map((tag) => (
              <button
                className="active-filter"
                key={tag}
                type="button"
                onClick={() =>
                  setSelectedTags((current) =>
                    current.filter((selected) => selected !== tag),
                  )
                }
              >
                {tag} <span aria-hidden="true">×</span>
                <span className="sr-only">Remove {tag} filter</span>
              </button>
            ))}
            <button
              className="clear-tag-filters"
              type="button"
              onClick={() => setSelectedTags([])}
            >
              Clear all
            </button>
          </div>
        )}
        {catalogContent}
      </div>
      {confirmUnsubscribeAll && (
        <ConfirmDialog
          title="Unsubscribe from every provider?"
          message="This removes all configured provider coverage. You can subscribe again at any time."
          confirmLabel="Unsubscribe all"
          pending={unsubscribeAll.isPending}
          onConfirm={() => unsubscribeAll.mutate()}
          onClose={() => setConfirmUnsubscribeAll(false)}
        />
      )}
    </>
  );
}

function CatalogCard({
  provider,
  monitor,
  selectedTags,
  onTagSelect,
}: {
  provider: CatalogProvider;
  monitor: Monitor | undefined;
  selectedTags: string[];
  onTagSelect: (tag: string) => void;
}) {
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const [expanded, setExpanded] = useState(false);
  const [coverageDraft, setCoverageDraft] = useState<{
    allComponents: boolean;
    selectedComponentIds: string[];
  }>();
  const [monitorIdToRemove, setMonitorIdToRemove] = useState<string>();

  const detail = useQuery({
    queryKey: queryKeys.catalogDetail(provider.id),
    queryFn: ({ signal }) =>
      api<CatalogDetail>(`/api/v1/catalog/providers/${provider.id}`, {
        signal,
      }),
    enabled: expanded,
  });

  const allComponents =
    coverageDraft?.allComponents ??
    detail.data?.monitor?.monitor_all_components ??
    true;
  const selectedComponentIds =
    coverageDraft?.selectedComponentIds ??
    (allComponents
      ? []
      : (detail.data?.components
          .filter((component) => component.selected)
          .map((component) => component.id) ?? []));

  const createMonitor = useMutation({
    mutationFn: (body: {
      monitor_all_components: boolean;
      component_ids: string[];
    }) =>
      api<Monitor>("/api/v1/monitors", {
        method: "POST",
        body: JSON.stringify({ provider_id: provider.id, ...body }),
      }),
    onSuccess: async (savedMonitor, coverage) => {
      notify(`Subscribed to ${provider.name}.`);
      queryClient.setQueryData<CatalogDetail>(
        queryKeys.catalogDetail(provider.id),
        (current) =>
          current && {
            ...current,
            monitor: savedMonitor,
            components: current.components.map((component) => ({
              ...component,
              selected:
                coverage.monitor_all_components ||
                coverage.component_ids.includes(component.id),
            })),
          },
      );
      setCoverageDraft(undefined);
      await invalidateMonitoringState(queryClient);
    },
    onError: (error) => notify(error.message, "error"),
  });

  const deleteMonitor = useMutation({
    mutationFn: (monitorId: string) =>
      api<void>(`/api/v1/monitors/${monitorId}`, { method: "DELETE" }),
    onSuccess: () => {
      notify(`Unsubscribed from ${provider.name}.`);
      setMonitorIdToRemove(undefined);
      refreshMonitoringState();
    },
    onError: (error) => notify(error.message, "error"),
  });

  const monitored = monitor?.enabled ?? false;

  function refreshMonitoringState() {
    void invalidateMonitoringState(queryClient);
  }

  function selectAllComponents(selected: boolean) {
    setCoverageDraft({
      allComponents: selected,
      selectedComponentIds: selected ? [] : selectedComponentIds,
    });
  }

  function selectComponent(componentId: string, selected: boolean) {
    setCoverageDraft({
      allComponents,
      selectedComponentIds: selected
        ? [...selectedComponentIds, componentId]
        : selectedComponentIds.filter((id) => id !== componentId),
    });
  }

  function monitorAllComponents() {
    createMonitor.mutate({
      monitor_all_components: true,
      component_ids: [],
    });
  }

  function monitorSelectedComponents() {
    createMonitor.mutate({
      monitor_all_components: false,
      component_ids: selectedComponentIds,
    });
  }

  return (
    <article className="card catalog-card">
      <div className="catalog-card-body">
        <div className="catalog-card-header">
          <div className="provider-title">
            <h2>
              <Link to={`/catalog/${provider.id}`}>{provider.name}</Link>
            </h2>
            {monitored && <span className="status-indicator">Subscribed</span>}
          </div>
          <p className="provider-description">{provider.description}</p>
        </div>
        <div className="catalog-actions">
          <button
            className="button ghost"
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? "Close" : "Configure coverage"}
          </button>
          {monitored ? (
            <button
              className="button danger"
              disabled={deleteMonitor.isPending}
              onClick={() => {
                deleteMonitor.reset();
                setMonitorIdToRemove(monitor?.id);
              }}
            >
              {deleteMonitor.isPending ? "Unsubscribing…" : "Unsubscribe"}
            </button>
          ) : (
            <button
              className="button primary"
              disabled={createMonitor.isPending}
              onClick={monitorAllComponents}
            >
              Subscribe
            </button>
          )}
        </div>
        {(createMonitor.isError || deleteMonitor.isError) && (
          <p className="alert error" role="alert">
            {(createMonitor.error ?? deleteMonitor.error)?.message}
          </p>
        )}
        {expanded && detail.isLoading && (
          <LoadingSkeleton
            label="Loading provider components"
            rows={2}
            inline
          />
        )}
        {expanded && detail.isError && (
          <p className="alert error" role="alert">
            Components could not be loaded.{" "}
            <button
              className="button ghost"
              onClick={() => void detail.refetch()}
            >
              Retry
            </button>
          </p>
        )}
        {expanded && detail.data && (
          <div className="component-picker">
            <div className="component-picker-heading">
              <div>
                <strong>Coverage</strong>
                <span className="muted">
                  Choose every component or a focused subset.
                </span>
              </div>
            </div>
            <label className="coverage-mode">
              Coverage mode
              <select
                value={allComponents ? "all" : "selected"}
                disabled={createMonitor.isPending}
                onChange={(event) =>
                  selectAllComponents(event.target.value === "all")
                }
              >
                <option value="all">All components</option>
                <option value="selected">Selected components</option>
              </select>
            </label>
            {!allComponents && (
              <>
                <div className="component-options">
                  {detail.data.components
                    .filter(
                      (component) =>
                        component.active ||
                        selectedComponentIds.includes(component.id),
                    )
                    .map((component) => (
                      <label
                        className="check component-option"
                        key={component.id}
                      >
                        <input
                          type="checkbox"
                          disabled={createMonitor.isPending}
                          checked={selectedComponentIds.includes(component.id)}
                          onChange={(event) =>
                            selectComponent(component.id, event.target.checked)
                          }
                        />
                        <span>
                          {component.group ? component.group + ": " : ""}
                          {component.name}
                          {!component.active && " (no longer reported)"}
                        </span>
                      </label>
                    ))}
                </div>
                <button
                  className="button primary"
                  disabled={
                    selectedComponentIds.length === 0 || createMonitor.isPending
                  }
                  onClick={monitorSelectedComponents}
                >
                  Save selected coverage
                </button>
              </>
            )}
            {allComponents &&
              monitored &&
              !detail.data.monitor?.monitor_all_components && (
                <button
                  className="button primary"
                  disabled={createMonitor.isPending}
                  onClick={monitorAllComponents}
                >
                  Save all components
                </button>
              )}
          </div>
        )}
        {monitorIdToRemove && (
          <ConfirmDialog
            title={`Unsubscribe from ${provider.name}?`}
            message="StatusDeck will stop monitoring this provider's configured coverage."
            confirmLabel="Unsubscribe"
            pending={deleteMonitor.isPending}
            error={deleteMonitor.error?.message}
            onConfirm={() => deleteMonitor.mutate(monitorIdToRemove)}
            onClose={() => setMonitorIdToRemove(undefined)}
          />
        )}
      </div>
      <div className="tag-list catalog-tags" aria-label="Provider categories">
        {provider.tags.map((tag) => (
          <button
            className={`tag${selectedTags.includes(tag) ? " active" : ""}`}
            key={tag}
            type="button"
            aria-pressed={selectedTags.includes(tag)}
            onClick={() => onTagSelect(tag)}
          >
            {tag}
          </button>
        ))}
      </div>
    </article>
  );
}
