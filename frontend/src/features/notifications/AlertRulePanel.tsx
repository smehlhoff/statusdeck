import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { createPortal } from "react-dom";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type {
  CatalogProvider,
  Channel,
  Monitor,
  Rule,
  RuleComponent,
  RuleKind,
  QuietHours,
  Severity,
} from "../../api/types";
import { QuietHoursFields } from "./QuietHoursFields";
import { humanizeIdentifier } from "../../utils/display";
import { requestMessage } from "./notificationUtils";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { useToast } from "../../components/toastContext";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";

type NotificationFlags = Pick<
  Rule,
  | "notify_detected"
  | "notify_started"
  | "notify_updated"
  | "notify_resolved"
  | "notify_reopened"
  | "notify_maintenance"
  | "notify_recovered"
>;

const NOTIFICATION_FLAG_KEYS = [
  "notify_detected",
  "notify_started",
  "notify_updated",
  "notify_resolved",
  "notify_reopened",
  "notify_maintenance",
  "notify_recovered",
] as const satisfies readonly (keyof NotificationFlags)[];

const INITIAL_FLAGS: NotificationFlags = {
  notify_detected: true,
  notify_started: true,
  notify_updated: true,
  notify_resolved: true,
  notify_reopened: true,
  notify_maintenance: false,
  notify_recovered: true,
};

interface RuleDraft {
  quietHours: QuietHours;
  name: string;
  kind: RuleKind;
  severity: Severity;
  providerIds: string[];
  componentIds: string[];
  channelIds: string[];
  flags: NotificationFlags;
}

const INITIAL_RULE_DRAFT: RuleDraft = {
  quietHours: {
    enabled: false,
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    days: [1, 2, 3, 4, 5, 6, 7],
    start: "22:00",
    end: "07:00",
    critical_override: false,
  },
  name: "",
  kind: "provider",
  severity: "minor",
  providerIds: [],
  componentIds: [],
  channelIds: [],
  flags: INITIAL_FLAGS,
};

const COPY_NAME_PATTERN = /^(.*) \(copy (\d+)\)$/i;

function copyName(base: string, number: number): string {
  const suffix = ` (copy ${number})`;
  return `${Array.from(base)
    .slice(0, 100 - suffix.length)
    .join("")
    .trimEnd()}${suffix}`;
}

function nextCopyName(name: string, rules: Rule[]): string {
  const base = name.match(COPY_NAME_PATTERN)?.[1] ?? name;
  let highestCopy = 0;
  for (const rule of rules) {
    const match = rule.name.match(COPY_NAME_PATTERN);
    if (!match) continue;
    const number = Number(match[2]);
    if (rule.name.toLowerCase() === copyName(base, number).toLowerCase()) {
      highestCopy = Math.max(highestCopy, number);
    }
  }
  return copyName(base, highestCopy + 1);
}

function toggleSelection(values: string[], value: string): string[] {
  return values.includes(value)
    ? values.filter((item) => item !== value)
    : [...values, value];
}

function draftFromRule(rule: Rule): RuleDraft {
  return {
    quietHours: rule.quiet_hours,
    name: rule.name,
    kind: rule.rule_kind,
    severity: rule.min_severity,
    providerIds: rule.provider_ids,
    componentIds: rule.component_ids,
    channelIds: rule.channel_ids,
    flags: {
      notify_detected: rule.notify_detected,
      notify_started: rule.notify_started,
      notify_updated: rule.notify_updated,
      notify_resolved: rule.notify_resolved,
      notify_reopened: rule.notify_reopened,
      notify_maintenance: rule.notify_maintenance,
      notify_recovered: rule.notify_recovered,
    },
  };
}

function requestFromDraft(draft: RuleDraft) {
  return {
    quiet_hours: draft.quietHours,
    name: draft.name,
    rule_kind: draft.kind,
    min_severity: draft.severity,
    channel_ids: draft.channelIds,
    provider_ids: draft.providerIds,
    component_ids: draft.kind === "provider" ? draft.componentIds : [],
    ...draft.flags,
  };
}

export function AlertRulePanel({
  highlightedChannelId,
  onDirtyChange,
}: {
  highlightedChannelId?: string;
  onDirtyChange: (dirty: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const channels = useQuery({
    queryKey: queryKeys.channels,
    queryFn: ({ signal }) =>
      api<Channel[]>("/api/v1/notification-channels", { signal }),
  });
  const rules = useQuery({
    queryKey: queryKeys.rules,
    queryFn: ({ signal }) => api<Rule[]>("/api/v1/alert-rules", { signal }),
  });
  const providers = useQuery({
    queryKey: queryKeys.catalog,
    queryFn: ({ signal }) =>
      api<CatalogProvider[]>("/api/v1/catalog/providers", { signal }),
  });
  const monitors = useQuery({
    queryKey: queryKeys.monitors,
    queryFn: ({ signal }) => api<Monitor[]>("/api/v1/monitors", { signal }),
  });

  const [draft, setDraft] = useState<RuleDraft>(INITIAL_RULE_DRAFT);
  const [providerSearch, setProviderSearch] = useState("");
  const [componentSearch, setComponentSearch] = useState<
    Record<string, string>
  >({});
  const [ruleError, setRuleError] = useState("");
  const [editingRuleId, setEditingRuleId] = useState<string | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [ruleToRemove, setRuleToRemove] = useState<Rule>();
  const [ruleToDisable, setRuleToDisable] = useState<Rule>();
  const [editorTarget, setEditorTarget] = useState<HTMLDivElement | null>(null);
  const originalDraft = editingRuleId
    ? rules.data?.find((rule) => rule.id === editingRuleId)
    : undefined;
  const ruleFormDirty =
    editorOpen &&
    (editingRuleId === null ||
      originalDraft === undefined ||
      JSON.stringify(draft) !== JSON.stringify(draftFromRule(originalDraft)));

  useEffect(() => {
    onDirtyChange(ruleFormDirty);
  }, [onDirtyChange, ruleFormDirty]);

  useEffect(() => () => onDirtyChange(false), [onDirtyChange]);

  useEffect(() => {
    if (!editorTarget) return;
    editorTarget.scrollIntoView({
      behavior: "smooth",
      block: editingRuleId === null ? "start" : "nearest",
    });
  }, [editingRuleId, editorTarget]);
  const isProviderRule = draft.kind === "provider";
  const monitoredProviderIds = new Set(
    monitors.data
      ?.filter((monitor) => monitor.enabled)
      .map((monitor) => monitor.provider_id) ?? [],
  );
  const normalizedProviderSearch = providerSearch.toLowerCase();
  const visibleProviders =
    providers.data?.filter((provider) =>
      provider.name.toLowerCase().includes(normalizedProviderSearch),
    ) ?? [];
  const selectedProviderIds = [...draft.providerIds].sort();
  const componentCatalog = useQuery({
    queryKey: queryKeys.catalogComponents(selectedProviderIds),
    queryFn: ({ signal }) => {
      const params = new URLSearchParams({
        provider_ids: selectedProviderIds.join(","),
      });
      return api<RuleComponent[]>(`/api/v1/catalog/components?${params}`, {
        signal,
      });
    },
    enabled: isProviderRule && draft.providerIds.length > 0,
  });
  const unavailableComponentIds =
    draft.providerIds.length === 0 || componentCatalog.isSuccess
      ? draft.componentIds.filter(
          (id) =>
            !componentCatalog.data?.some((component) => component.id === id),
        )
      : [];

  const saveRule = useMutation({
    mutationFn: () =>
      api<Rule>(
        editingRuleId
          ? `/api/v1/alert-rules/${editingRuleId}`
          : "/api/v1/alert-rules",
        {
          method: editingRuleId ? "PATCH" : "POST",
          body: JSON.stringify(requestFromDraft(draft)),
        },
      ),
    onSuccess: () => {
      notify(editingRuleId ? "Alert rule updated." : "Alert rule created.");
      resetRuleForm();
      void queryClient.invalidateQueries({ queryKey: queryKeys.rules });
    },
    onError: (error) => {
      setRuleError(requestMessage(error));
    },
  });

  const updateRule = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      api<Rule>(`/api/v1/alert-rules/${id}`, {
        method: "PATCH",
        body: JSON.stringify({ enabled }),
      }),
    onSuccess: () => {
      notify("Alert rule status updated.");
      setRuleToDisable(undefined);
      setRuleError("");
      void queryClient.invalidateQueries({ queryKey: queryKeys.rules });
      void queryClient.invalidateQueries({ queryKey: queryKeys.deliveries });
    },
    onError: (error) => {
      notify(requestMessage(error), "error");
    },
  });

  const deleteRule = useMutation({
    mutationFn: (id: string) =>
      api<void>(`/api/v1/alert-rules/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      notify("Alert rule removed.");
      setRuleToRemove(undefined);
      setRuleError("");
      void queryClient.invalidateQueries({ queryKey: queryKeys.rules });
    },
  });

  function submitRule(event: FormEvent) {
    event.preventDefault();
    if (saveRule.isPending) return;
    setRuleError("");
    saveRule.mutate();
  }

  function editRule(rule: Rule) {
    setComponentSearch({});
    setEditingRuleId(rule.id);
    setDraft(draftFromRule(rule));
    setRuleError("");
    setEditorOpen(true);
  }

  function cloneRule(rule: Rule) {
    setComponentSearch({});
    setEditingRuleId(null);
    setDraft({
      ...draftFromRule(rule),
      name: nextCopyName(rule.name, rules.data ?? []),
    });
    setProviderSearch("");
    setRuleError("");
    setEditorOpen(true);
  }

  function saveButtonLabel() {
    if (saveRule.isPending) return "Saving…";
    if (editingRuleId) return "Save changes";
    return "Add rule";
  }

  function resetRuleForm() {
    setEditingRuleId(null);
    setDraft(INITIAL_RULE_DRAFT);
    setRuleError("");
    setEditorOpen(false);
  }

  function startRuleForm() {
    setComponentSearch({});
    setEditingRuleId(null);
    setDraft(INITIAL_RULE_DRAFT);
    setRuleError("");
    setEditorOpen(true);
  }

  return (
    <section className="card alert-rules-panel" id="configured-alert-rules">
      <div className="section-heading">
        <div>
          <h2>Configured alert rules</h2>
          <p className="section-description">
            Rules connect monitored events to a delivery channel.
          </p>
        </div>
        {!editorOpen && (
          <button
            className="button ghost"
            type="button"
            onClick={startRuleForm}
          >
            New rule
          </button>
        )}
      </div>
      {((channels.isError && !channels.data) ||
        (rules.isError && !rules.data) ||
        (providers.isError && !providers.data) ||
        (monitors.isError && !monitors.data)) && (
        <p className="alert error" role="alert">
          Alert-rule configuration could not be loaded.{" "}
          <button
            className="button ghost"
            onClick={() => {
              void channels.refetch();
              void rules.refetch();
              void providers.refetch();
              void monitors.refetch();
            }}
          >
            Retry
          </button>
        </p>
      )}
      {rules.isLoading && (
        <LoadingSkeleton label="Loading alert rules" inline />
      )}
      {rules.data && rules.data.length > 0 && (
        <ul className="plain-list rule-list">
          {rules.data.map((rule) => (
            <li
              className={
                highlightedChannelId &&
                rule.channel_ids.includes(highlightedChannelId)
                  ? "rule-highlighted"
                  : undefined
              }
              key={rule.id}
            >
              <div className="rule-list-summary">
                <div>
                  <strong>{rule.name}</strong>
                  <span>
                    {humanizeIdentifier(rule.rule_kind)} · minimum{" "}
                    {rule.min_severity}
                  </span>
                  {rule.quiet_hours.enabled && (
                    <span>
                      Quiet hours {rule.quiet_hours.start}–
                      {rule.quiet_hours.end} · {rule.quiet_hours.timezone}
                    </span>
                  )}
                </div>
                <span
                  className={`badge ${rule.enabled ? "operational" : "neutral"}`}
                >
                  {rule.enabled ? "Enabled" : "Disabled"}
                </span>
              </div>
              <div className="card-actions">
                <button
                  className="button ghost"
                  disabled={saveRule.isPending}
                  onClick={() => editRule(rule)}
                >
                  Edit
                </button>
                <button
                  className="button ghost"
                  disabled={updateRule.isPending || saveRule.isPending}
                  onClick={() => {
                    if (rule.enabled) {
                      updateRule.reset();
                      setRuleToDisable(rule);
                    } else updateRule.mutate({ id: rule.id, enabled: true });
                  }}
                >
                  {updateRule.isPending &&
                    updateRule.variables?.id === rule.id &&
                    "Updating…"}
                  {(!updateRule.isPending ||
                    updateRule.variables?.id !== rule.id) &&
                    (rule.enabled ? "Disable" : "Enable")}
                </button>
                <button
                  className="button ghost"
                  type="button"
                  disabled={saveRule.isPending}
                  onClick={() => cloneRule(rule)}
                >
                  Clone
                </button>
                <button
                  className="button ghost"
                  disabled={saveRule.isPending}
                  onClick={() => {
                    deleteRule.reset();
                    setRuleToRemove(rule);
                  }}
                >
                  Remove
                </button>
              </div>
              {editingRuleId === rule.id && (
                <div className="rule-editor-slot" ref={setEditorTarget} />
              )}
            </li>
          ))}
        </ul>
      )}
      {rules.data && rules.data.length === 0 && (
        <EmptyState
          title="No alert rules"
          description="Create a rule to connect monitored provider events to a delivery channel."
          inline
          action={
            !editorOpen ? (
              <button
                className="button ghost"
                type="button"
                onClick={startRuleForm}
              >
                Create alert rule
              </button>
            ) : undefined
          }
        />
      )}
      {editorOpen && editingRuleId === null && (
        <div className="rule-editor-slot" ref={setEditorTarget} />
      )}
      {editorOpen &&
        editorTarget &&
        createPortal(
          <div className="rule-editor">
            <h3>{editingRuleId ? "Edit alert rule" : "New alert rule"}</h3>
            <form onSubmit={submitRule}>
              <div className="rule-basics">
                <label>
                  Name
                  <input
                    autoFocus
                    required
                    value={draft.name}
                    disabled={saveRule.isPending}
                    onChange={(event) =>
                      setDraft((current) => ({
                        ...current,
                        name: event.target.value,
                      }))
                    }
                  />
                </label>
                <label>
                  Rule mode
                  <select
                    value={draft.kind}
                    disabled={saveRule.isPending}
                    onChange={(event) => {
                      setDraft((current) => ({
                        ...current,
                        kind: event.target.value as RuleKind,
                        providerIds: [],
                        componentIds: [],
                      }));
                    }}
                  >
                    <option value="provider">Provider events</option>
                    <option value="system_health">System health</option>
                  </select>
                </label>
                {isProviderRule && (
                  <label>
                    Minimum severity
                    <select
                      value={draft.severity}
                      disabled={saveRule.isPending}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          severity: event.target.value as Severity,
                        }))
                      }
                    >
                      <option value="info">Info</option>
                      <option value="minor">Minor</option>
                      <option value="major">Major</option>
                      <option value="critical">Critical</option>
                    </select>
                  </label>
                )}
              </div>
              <fieldset
                className="selection-fieldset"
                disabled={saveRule.isPending}
              >
                <legend>
                  {isProviderRule
                    ? `Providers · ${draft.providerIds.length} selected`
                    : `Sources through providers · ${draft.providerIds.length} selected`}
                </legend>
                <div className="selection-toolbar">
                  <input
                    aria-label="Filter providers"
                    value={providerSearch}
                    onChange={(event) => setProviderSearch(event.target.value)}
                    placeholder="Filter providers"
                  />
                  <button
                    className="button ghost"
                    type="button"
                    onClick={() =>
                      setDraft((current) => ({
                        ...current,
                        providerIds:
                          providers.data
                            ?.filter((provider) =>
                              monitoredProviderIds.has(provider.id),
                            )
                            .map((provider) => provider.id) ?? [],
                      }))
                    }
                  >
                    Select all
                  </button>
                  <button
                    className="button ghost"
                    type="button"
                    onClick={() =>
                      setDraft((current) => ({
                        ...current,
                        providerIds: [],
                        componentIds: [],
                      }))
                    }
                  >
                    Clear
                  </button>
                </div>
                {!isProviderRule && (
                  <p className="muted">
                    Leave empty for every monitored source.
                  </p>
                )}
                <div className="provider-choice-grid">
                  {visibleProviders.map((provider) => (
                    <label
                      className="check choice-card"
                      key={provider.id}
                      title={
                        monitoredProviderIds.has(provider.id)
                          ? undefined
                          : "Subscribe to this provider before using it in a rule."
                      }
                    >
                      <input
                        type="checkbox"
                        disabled={
                          (!monitoredProviderIds.has(provider.id) &&
                            !draft.providerIds.includes(provider.id)) ||
                          (isProviderRule &&
                            draft.providerIds.length > 0 &&
                            componentCatalog.isFetching)
                        }
                        checked={draft.providerIds.includes(provider.id)}
                        onChange={() =>
                          setDraft((current) => {
                            const removing = current.providerIds.includes(
                              provider.id,
                            );
                            return {
                              ...current,
                              providerIds: toggleSelection(
                                current.providerIds,
                                provider.id,
                              ),
                              componentIds: removing
                                ? current.componentIds.filter((componentId) => {
                                    const component =
                                      componentCatalog.data?.find(
                                        (component) =>
                                          component.id === componentId,
                                      );
                                    return (
                                      component === undefined ||
                                      component.provider_id !== provider.id
                                    );
                                  })
                                : current.componentIds,
                            };
                          })
                        }
                      />
                      <span>
                        {provider.name}
                        {!monitoredProviderIds.has(provider.id) && (
                          <small>Not subscribed</small>
                        )}
                      </span>
                    </label>
                  ))}
                </div>
              </fieldset>
              {isProviderRule && componentCatalog.isLoading && (
                <LoadingSkeleton
                  label="Loading provider components"
                  rows={2}
                  inline
                />
              )}
              {isProviderRule &&
                draft.providerIds.map((providerId) => {
                  const provider = providers.data?.find(
                    (candidate) => candidate.id === providerId,
                  );
                  const components =
                    componentCatalog.data?.filter(
                      (component) => component.provider_id === providerId,
                    ) ?? [];
                  const search = (componentSearch[providerId] ?? "")
                    .trim()
                    .toLowerCase();
                  const visibleComponents = components.filter((component) =>
                    `${component.group ?? ""} ${component.name}`
                      .toLowerCase()
                      .includes(search),
                  );
                  const selectableComponentIds = components
                    .filter((component) => component.monitored)
                    .map((component) => component.id);
                  const selectedComponentCount = components.filter(
                    (component) => draft.componentIds.includes(component.id),
                  ).length;
                  return (
                    <details className="component-selection" key={providerId}>
                      <summary>
                        {provider?.name ?? "Provider"} components
                        <span className="component-selection-actions">
                          <span className="component-selection-count">
                            {selectedComponentCount} selected
                          </span>
                          <button
                            className="button ghost"
                            type="button"
                            aria-label={`Select all monitored components for ${provider?.name ?? "provider"}`}
                            disabled={
                              saveRule.isPending ||
                              componentCatalog.isFetching ||
                              selectableComponentIds.every((id) =>
                                draft.componentIds.includes(id),
                              )
                            }
                            onClick={(event) => {
                              event.preventDefault();
                              setDraft((current) => ({
                                ...current,
                                componentIds: [
                                  ...new Set([
                                    ...current.componentIds,
                                    ...selectableComponentIds,
                                  ]),
                                ],
                              }));
                            }}
                          >
                            Select all
                          </button>
                          <button
                            className="button ghost"
                            type="button"
                            aria-label={`Remove all selected components for ${provider?.name ?? "provider"}`}
                            disabled={
                              saveRule.isPending ||
                              componentCatalog.isFetching ||
                              selectedComponentCount === 0
                            }
                            onClick={(event) => {
                              event.preventDefault();
                              const providerComponentIds = new Set(
                                components.map((component) => component.id),
                              );
                              setDraft((current) => ({
                                ...current,
                                componentIds: current.componentIds.filter(
                                  (id) => !providerComponentIds.has(id),
                                ),
                              }));
                            }}
                          >
                            Remove all
                          </button>
                        </span>
                      </summary>
                      <div className="selection-toolbar">
                        <input
                          type="search"
                          aria-label={`Filter components for ${provider?.name ?? "provider"}`}
                          placeholder="Filter components by name or region"
                          value={componentSearch[providerId] ?? ""}
                          disabled={saveRule.isPending}
                          onChange={(event) =>
                            setComponentSearch((current) => ({
                              ...current,
                              [providerId]: event.target.value,
                            }))
                          }
                        />
                      </div>
                      <div className="provider-choice-grid">
                        {search &&
                          componentCatalog.isSuccess &&
                          visibleComponents.length === 0 && (
                            <p className="muted" role="status">
                              No components match your search.
                            </p>
                          )}
                        {visibleComponents.map((component) => (
                          <label
                            className="check choice-card"
                            key={component.id}
                          >
                            <input
                              type="checkbox"
                              disabled={
                                saveRule.isPending ||
                                (!component.monitored &&
                                  !draft.componentIds.includes(component.id))
                              }
                              checked={draft.componentIds.includes(
                                component.id,
                              )}
                              onChange={() =>
                                setDraft((current) => ({
                                  ...current,
                                  componentIds: toggleSelection(
                                    current.componentIds,
                                    component.id,
                                  ),
                                }))
                              }
                            />
                            {component.group ? `${component.group}: ` : ""}
                            {component.name}
                            {!component.monitored && " (not monitored)"}
                          </label>
                        ))}
                      </div>
                    </details>
                  );
                })}
              {isProviderRule && componentCatalog.isError && (
                <p className="alert error" role="alert">
                  Components could not be loaded.{" "}
                  <button
                    className="button ghost"
                    type="button"
                    onClick={() => void componentCatalog.refetch()}
                  >
                    Retry
                  </button>
                </p>
              )}
              {isProviderRule && unavailableComponentIds.length > 0 && (
                <p className="alert" role="status">
                  {unavailableComponentIds.length} selected component(s) are no
                  longer available for these providers.{" "}
                  <button
                    className="button ghost"
                    type="button"
                    disabled={saveRule.isPending}
                    onClick={() =>
                      setDraft((current) => ({
                        ...current,
                        componentIds: current.componentIds.filter(
                          (id) => !unavailableComponentIds.includes(id),
                        ),
                      }))
                    }
                  >
                    Remove unavailable selections
                  </button>
                </p>
              )}
              {!isProviderRule && (
                <p className="muted">
                  Monitoring failure remains separate from provider outage
                  severity.
                </p>
              )}
              <fieldset
                className="event-choice-fieldset"
                disabled={saveRule.isPending}
              >
                <legend>Events</legend>
                <div className="event-choice-grid">
                  {NOTIFICATION_FLAG_KEYS.map((key) => (
                    <label className="check" key={key}>
                      <input
                        type="checkbox"
                        checked={draft.flags[key]}
                        onChange={() =>
                          setDraft((current) => ({
                            ...current,
                            flags: {
                              ...current.flags,
                              [key]: !current.flags[key],
                            },
                          }))
                        }
                      />
                      {humanizeIdentifier(key.replace("notify_", ""))}
                    </label>
                  ))}
                </div>
              </fieldset>
              <QuietHoursFields
                value={draft.quietHours}
                disabled={saveRule.isPending}
                onChange={(quietHours) =>
                  setDraft((current) => ({ ...current, quietHours }))
                }
              />
              <fieldset disabled={saveRule.isPending}>
                <legend>Channels</legend>
                <p className="fieldset-help">
                  Choose where matching alerts are sent. Create a delivery
                  channel first if none appear here.
                </p>
                {channels.isSuccess && channels.data.length === 0 && (
                  <p className="fieldset-help">
                    No delivery channels configured.
                  </p>
                )}
                {channels.data?.map((channel) => {
                  const selected = draft.channelIds.includes(channel.id);
                  return (
                    <label className="check" key={channel.id}>
                      <input
                        type="checkbox"
                        disabled={!channel.enabled && !selected}
                        checked={selected}
                        onChange={() =>
                          setDraft((current) => ({
                            ...current,
                            channelIds: toggleSelection(
                              current.channelIds,
                              channel.id,
                            ),
                          }))
                        }
                      />
                      <span>
                        {channel.name}
                        {!channel.enabled && (
                          <small>Disabled — retained but not sent</small>
                        )}
                      </span>
                    </label>
                  );
                })}
              </fieldset>
              {ruleError && (
                <p className="alert error" role="alert">
                  {ruleError}
                </p>
              )}
              <div className="rule-editor-actions">
                <button
                  className="button primary compact"
                  disabled={
                    saveRule.isPending ||
                    (draft.quietHours.enabled &&
                      (!draft.quietHours.days.length ||
                        draft.quietHours.start === draft.quietHours.end)) ||
                    !draft.channelIds.length ||
                    (isProviderRule && !draft.providerIds.length)
                  }
                >
                  {saveButtonLabel()}
                </button>
                <button
                  type="button"
                  className="button ghost compact"
                  onClick={resetRuleForm}
                  disabled={saveRule.isPending}
                >
                  {editingRuleId ? "Cancel edit" : "Cancel"}
                </button>
              </div>
            </form>
          </div>,
          editorTarget,
        )}
      {ruleToRemove && (
        <ConfirmDialog
          title={`Remove alert rule “${ruleToRemove.name}”?`}
          message="Matching events will no longer be sent through this rule."
          confirmLabel="Remove rule"
          pending={deleteRule.isPending}
          error={deleteRule.error?.message}
          onConfirm={() => deleteRule.mutate(ruleToRemove.id)}
          onClose={() => setRuleToRemove(undefined)}
        />
      )}
      {ruleToDisable && (
        <ConfirmDialog
          title={`Disable alert rule “${ruleToDisable.name}”?`}
          message="Automatically queued, retrying, and held deliveries created by this rule will be cancelled. Re-enabling applies only to new events. A delivery already in flight may still complete."
          confirmLabel="Disable rule"
          pending={updateRule.isPending}
          error={updateRule.error?.message}
          onConfirm={() =>
            updateRule.mutate({ id: ruleToDisable.id, enabled: false })
          }
          onClose={() => setRuleToDisable(undefined)}
        />
      )}
    </section>
  );
}
