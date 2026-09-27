import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Link,
  useBeforeUnload,
  useBlocker,
  useNavigate,
  useParams,
} from "react-router-dom";
import { api } from "../../api/client";
import { invalidateMonitoringState, queryKeys } from "../../api/queries";
import type { CatalogDetail, Monitor } from "../../api/types";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { useToast } from "../../components/toastContext";
import { CoveragePicker } from "./CoveragePicker";
import "./coverage.css";

type CoverageMode = "all" | "selected";

export function Coverage() {
  const { id } = useParams();
  const [loadedProviderId, setLoadedProviderId] = useState<string>();
  const detail = useQuery({
    queryKey: queryKeys.catalogDetail(id),
    refetchOnMount: "always",
    queryFn: ({ signal }) =>
      api<CatalogDetail>(`/api/v1/catalog/providers/${id}`, { signal }),
  });

  useEffect(() => {
    if (detail.isSuccess && !detail.isFetching) setLoadedProviderId(id);
  }, [detail.isSuccess, detail.isFetching, id]);

  // Start a draft from a fresh response, not a stale detail cached before a subscription change.
  if (detail.isLoading || (detail.isFetching && loadedProviderId !== id))
    return <LoadingSkeleton label="Loading coverage" rows={4} />;
  if (!detail.data || (detail.isError && loadedProviderId !== id))
    return (
      <>
        <EmptyState
          title="Coverage unavailable"
          description={
            detail.error?.message ?? "This provider could not be loaded."
          }
          error
          action={
            <button
              type="button"
              className="button ghost"
              onClick={() => void detail.refetch()}
            >
              Retry
            </button>
          }
        />
      </>
    );
  return (
    <CoverageEditor
      key={detail.data.id}
      provider={detail.data}
      refreshError={detail.isError}
    />
  );
}

function CoverageEditor({
  provider,
  refreshError,
}: {
  provider: CatalogDetail;
  refreshError: boolean;
}) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { notify } = useToast();
  const [original] = useState(() => ({
    mode: (provider.monitor?.monitor_all_components !== false
      ? "all"
      : "selected") as CoverageMode,
    ids: new Set(
      provider.monitor?.monitor_all_components === false
        ? provider.components
            .filter((item) => item.selected)
            .map((item) => item.id)
        : [],
    ),
  }));
  const [mode, setMode] = useState(original.mode);
  const [selectedIds, setSelectedIds] = useState(original.ids);
  const saved = useRef(false);
  const dirty =
    mode !== original.mode ||
    (mode === "selected" &&
      (selectedIds.size !== original.ids.size ||
        [...selectedIds].some((id) => !original.ids.has(id))));
  const components = provider.components.filter(
    (item) =>
      item.active ||
      (mode === "selected" &&
        (original.ids.has(item.id) || selectedIds.has(item.id))),
  );
  const effectiveIds =
    mode === "all" ? new Set(components.map((item) => item.id)) : selectedIds;
  const selectedComponents = components.filter((item) =>
    effectiveIds.has(item.id),
  );
  const selectedGroups = new Set(
    selectedComponents.flatMap((item) => (item.group ? [item.group] : [])),
  );
  const selectedServices = new Set(selectedComponents.map((item) => item.name));

  const save = useMutation({
    mutationFn: () =>
      api<Monitor>("/api/v1/monitors", {
        method: "POST",
        body: JSON.stringify({
          provider_id: provider.id,
          monitor_all_components: mode === "all",
          component_ids: mode === "all" ? [] : [...selectedIds],
        }),
      }),
    onSuccess: async () => {
      saved.current = true;
      await invalidateMonitoringState(queryClient);
      notify(`Coverage saved for ${provider.name}.`);
      navigate("/catalog", { replace: true });
    },
    onError: (error) => notify(error.message, "error"),
  });
  const needsProtection = dirty || save.isPending;
  const blocker = useBlocker(() => needsProtection && !saved.current);
  useBeforeUnload(
    useCallback(
      (event) => {
        if (!needsProtection || saved.current) return;
        event.preventDefault();
        event.returnValue = "";
      },
      [needsProtection],
    ),
  );

  function select(ids: string[], selected: boolean) {
    setSelectedIds((current) => {
      const next = new Set(current);
      for (const id of ids) {
        if (selected) next.add(id);
        else next.delete(id);
      }
      return next;
    });
  }

  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Configure Coverage</h1>
          <p className="muted">{provider.name}</p>
        </div>
      </div>
      {refreshError && (
        <p className="alert error" role="alert">
          Coverage could not be refreshed. Showing the last loaded catalog.
        </p>
      )}
      <form
        className="card coverage-editor"
        onSubmit={(event) => {
          event.preventDefault();
          if (!save.isPending && (mode === "all" || selectedIds.size > 0))
            save.mutate();
        }}
      >
        <div className="coverage-heading">
          <fieldset className="coverage-modes" disabled={save.isPending}>
            <legend className="sr-only">Coverage mode</legend>
            {(
              [
                ["all", "All components"],
                ["selected", "Selected components"],
              ] as const
            ).map(([value, label]) => (
              <label key={value} className="choice-card check">
                <input
                  type="radio"
                  name="coverage-mode"
                  value={value}
                  checked={mode === value}
                  onChange={() => setMode(value)}
                />
                {label}
              </label>
            ))}
          </fieldset>
        </div>
        <div
          className={`coverage-workspace${mode === "all" ? " coverage-all" : ""}`}
        >
          <CoveragePicker
            components={components}
            selectedIds={effectiveIds}
            hidden={mode === "all"}
            disabled={save.isPending}
            onSelect={select}
          />
          <aside className="coverage-summary" aria-label="Your coverage">
            <h2>Your coverage</h2>
            <p className="coverage-total" role="status">
              <strong>{effectiveIds.size.toLocaleString()}</strong> component
              {effectiveIds.size === 1 ? "" : "s"}
            </p>
            <p className="muted coverage-summary-counts">
              Services: {selectedServices.size} · Regions / groups:{" "}
              {selectedGroups.size}
            </p>
            <p className="muted">
              {mode === "all"
                ? "New components are automatically included."
                : "Only your selected components are included."}
            </p>
            {mode === "selected" && selectedComponents.length > 0 && (
              <ul
                className="coverage-selected-list"
                aria-label="Selected components"
                tabIndex={0}
              >
                {selectedComponents
                  .sort(
                    (a, b) =>
                      a.name.localeCompare(b.name) ||
                      (a.group ?? "").localeCompare(b.group ?? ""),
                  )
                  .map((item) => (
                    <li key={item.id}>
                      {item.name}
                      {item.group && (
                        <small className="muted">{item.group}</small>
                      )}
                      {!item.active && (
                        <small className="muted">
                          No longer reported by provider
                        </small>
                      )}
                    </li>
                  ))}
              </ul>
            )}
          </aside>
        </div>
        <div className="coverage-footer">
          {(save.isError ||
            (mode === "selected" && selectedIds.size === 0)) && (
            <div>
              {save.isError ? (
                <p className="coverage-save-error" role="alert">
                  {save.error.message}
                </p>
              ) : (
                <span className="muted">
                  Select at least one component to save.
                </span>
              )}
            </div>
          )}
          <Link className="button ghost" to="/catalog">
            Cancel
          </Link>
          <button
            className="button primary"
            type="submit"
            disabled={
              save.isPending || (mode === "selected" && selectedIds.size === 0)
            }
          >
            {save.isPending ? "Saving…" : "Save coverage"}
          </button>
        </div>
      </form>
      {blocker.state === "blocked" && (
        <ConfirmDialog
          title="Discard unsaved coverage?"
          message={
            save.isPending
              ? "Wait for coverage to finish saving."
              : "Your coverage changes have not been saved."
          }
          confirmLabel="Discard changes"
          pending={save.isPending}
          onConfirm={blocker.proceed}
          onClose={blocker.reset}
        />
      )}
    </>
  );
}
