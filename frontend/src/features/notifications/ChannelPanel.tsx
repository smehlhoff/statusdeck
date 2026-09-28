import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useState, type FormEvent } from "react";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { Channel, ChannelType, Rule } from "../../api/types";
import { requestMessage } from "./notificationUtils";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { useToast } from "../../components/toastContext";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { StatusBadge } from "../../components/StatusBadge";
import { FieldHelp } from "../../components/FieldHelp";

interface ChannelEditDraft {
  id: string;
  originalName: string;
  name: string;
  target: string;
  signingSecret: string;
  token: string;
  bot_email: string;
  stream: string;
  topic: string;
}

interface ChannelPanelProps {
  highlightedChannelId?: string;
  onDirtyChange: (dirty: boolean) => void;
  onHighlightRules: (channelId: string) => void;
}

const channelLabels: Record<ChannelType, string> = {
  webhook: "Generic webhook",
  discord: "Discord",
  slack: "Slack",
  mattermost: "Mattermost",
  gotify: "Gotify",
  ntfy: "ntfy.sh",
  zulip: "Zulip",
};

const channelTypes: ChannelType[] = [
  "webhook",
  "discord",
  "gotify",
  "mattermost",
  "ntfy",
  "slack",
  "zulip",
];

const targetPlaceholders: Record<ChannelType, string> = {
  webhook: "https://example.com/statusdeck",
  discord: "https://discord.com/api/webhooks/…",
  slack: "https://hooks.slack.com/services/…",
  mattermost: "https://mattermost.example.com/hooks/…",
  gotify: "https://gotify.example.com/message",
  ntfy: "https://ntfy.sh/statusdeck-topic",
  zulip: "https://zulip.example.com/api/v1/messages",
};

const tokenLabels: Partial<Record<ChannelType, string>> = {
  gotify: "Gotify application token",
  ntfy: "ntfy access token",
  zulip: "Zulip bot API key",
};

const zulipFieldLabels = {
  bot_email: "Zulip bot email",
  stream: "Zulip channel name",
  topic: "Zulip topic",
} as const;

export function ChannelPanel({
  highlightedChannelId,
  onDirtyChange,
  onHighlightRules,
}: ChannelPanelProps) {
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
  const ruleUsage = new Map<string, number>();
  for (const rule of rules.data ?? []) {
    for (const channelId of rule.channel_ids) {
      ruleUsage.set(channelId, (ruleUsage.get(channelId) ?? 0) + 1);
    }
  }

  const [channelName, setChannelName] = useState("");
  const [channelType, setChannelType] = useState<ChannelType>("webhook");
  const [target, setTarget] = useState("");
  const [signingSecret, setSigningSecret] = useState("");
  const [token, setToken] = useState("");
  const [zulip, setZulip] = useState({ bot_email: "", stream: "", topic: "" });
  const [editDraft, setEditDraft] = useState<ChannelEditDraft | null>(null);
  const [channelError, setChannelError] = useState("");
  const [channelToRemove, setChannelToRemove] = useState<Channel>();
  const [channelToDisable, setChannelToDisable] = useState<Channel>();
  const createFormDirty = Boolean(
    channelName ||
    target ||
    signingSecret ||
    token ||
    Object.values(zulip).some(Boolean),
  );
  const editFormDirty = Boolean(
    editDraft &&
    (editDraft.name !== editDraft.originalName ||
      editDraft.target ||
      editDraft.signingSecret ||
      editDraft.token ||
      editDraft.bot_email ||
      editDraft.stream ||
      editDraft.topic),
  );

  useEffect(() => {
    onDirtyChange(createFormDirty || editFormDirty);
  }, [createFormDirty, editFormDirty, onDirtyChange]);

  useEffect(() => () => onDirtyChange(false), [onDirtyChange]);

  useEffect(() => {
    function closeChannelMenus(event: PointerEvent) {
      if (!(event.target instanceof Node)) return;
      document
        .querySelectorAll<HTMLDetailsElement>(".channel-menu[open]")
        .forEach((menu) => {
          if (!menu.contains(event.target as Node)) menu.open = false;
        });
    }

    document.addEventListener("pointerdown", closeChannelMenus);
    return () => document.removeEventListener("pointerdown", closeChannelMenus);
  }, []);

  const createChannel = useMutation({
    mutationFn: () =>
      api<Channel>("/api/v1/notification-channels", {
        method: "POST",
        body: JSON.stringify({
          name: channelName,
          channel_type: channelType,
          target,
          ...(channelType === "zulip" ? zulip : {}),
          ...(channelType === "webhook" && signingSecret
            ? { signing_secret: signingSecret }
            : {}),
          ...((channelType === "gotify" ||
            channelType === "ntfy" ||
            channelType === "zulip") &&
          token
            ? { token }
            : {}),
        }),
      }),
    onSuccess: () => {
      notify("Delivery channel created.");
      setChannelName("");
      setTarget("");
      setSigningSecret("");
      setToken("");
      setZulip({ bot_email: "", stream: "", topic: "" });
      setChannelError("");
      void queryClient.invalidateQueries({ queryKey: queryKeys.channels });
    },
    onError: (error) => {
      setChannelError(requestMessage(error));
    },
  });

  const updateChannel = useMutation({
    mutationFn: (input: {
      id: string;
      name: string;
      target?: string;
      signingSecret?: string;
      token?: string;
      bot_email?: string;
      stream?: string;
      topic?: string;
      enabled?: boolean;
    }) =>
      api<Channel>(`/api/v1/notification-channels/${input.id}`, {
        method: "PATCH",
        body: JSON.stringify({
          name: input.name,
          ...(input.target ? { target: input.target } : {}),
          ...(input.signingSecret
            ? { signing_secret: input.signingSecret }
            : {}),
          ...(input.token ? { token: input.token } : {}),
          ...(input.bot_email ? { bot_email: input.bot_email } : {}),
          ...(input.stream ? { stream: input.stream } : {}),
          ...(input.topic ? { topic: input.topic } : {}),
          ...(input.enabled === undefined ? {} : { enabled: input.enabled }),
        }),
      }),
    onSuccess: (_channel, input) => {
      notify("Delivery channel updated.");
      setChannelToDisable(undefined);
      setEditDraft((current) => (current?.id === input.id ? null : current));
      setChannelError("");
      void queryClient.invalidateQueries({ queryKey: queryKeys.channels });
      if (input.enabled !== undefined)
        void queryClient.invalidateQueries({ queryKey: queryKeys.deliveries });
    },
    onError: (error, input) => {
      const message = requestMessage(error);
      if (input.enabled === undefined) setChannelError(message);
      else notify(message, "error");
    },
  });

  const testChannel = useMutation({
    mutationFn: (id: string) =>
      api<{ status: string }>(`/api/v1/notification-channels/${id}/test`, {
        method: "POST",
      }),
    onSuccess: (result) => {
      notify(`Channel test ${result.status}.`);
      void queryClient.invalidateQueries({ queryKey: queryKeys.deliveries });
    },
    onError: (error) => notify(requestMessage(error), "error"),
  });

  const deleteChannel = useMutation({
    mutationFn: (id: string) =>
      api<void>(`/api/v1/notification-channels/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      notify("Delivery channel removed.");
      setChannelToRemove(undefined);
      setChannelError("");
      void queryClient.invalidateQueries({ queryKey: queryKeys.channels });
    },
  });

  function submitChannel(event: FormEvent) {
    event.preventDefault();
    if (createChannel.isPending) return;
    setChannelError("");
    createChannel.mutate();
  }

  function editChannel(channel: Channel) {
    setEditDraft({
      id: channel.id,
      originalName: channel.name,
      name: channel.name,
      target: "",
      signingSecret: "",
      token: "",
      bot_email: "",
      stream: "",
      topic: "",
    });
    setChannelError("");
  }

  function updateEditDraft(
    update: Partial<Omit<ChannelEditDraft, "id" | "originalName">>,
  ) {
    setEditDraft((current) => (current ? { ...current, ...update } : current));
  }

  function cancelChannelEdit() {
    setEditDraft(null);
    setChannelError("");
  }

  return (
    <section className="card">
      <h2>Delivery channels</h2>
      <p className="section-description">
        A channel is an HTTPS webhook destination where matching alert rules
        send notifications.
      </p>
      <form onSubmit={submitChannel}>
        <label>
          Name
          <input
            required
            value={channelName}
            disabled={createChannel.isPending}
            onChange={(event) => setChannelName(event.target.value)}
          />
        </label>
        <label>
          Type
          <select
            value={channelType}
            disabled={createChannel.isPending}
            onChange={(event) =>
              setChannelType(event.target.value as ChannelType)
            }
          >
            {channelTypes.map((type) => (
              <option key={type} value={type}>
                {channelLabels[type]}
              </option>
            ))}
          </select>
        </label>
        <label>
          HTTPS destination
          <input
            required
            type="url"
            value={target}
            disabled={createChannel.isPending}
            onChange={(event) => setTarget(event.target.value)}
            placeholder={targetPlaceholders[channelType]}
          />
        </label>
        {channelType === "webhook" && (
          <SigningSecretField
            value={signingSecret}
            disabled={createChannel.isPending}
            onChange={setSigningSecret}
            placeholder="Add a secret to sign webhook requests"
          />
        )}
        {(channelType === "gotify" ||
          channelType === "ntfy" ||
          channelType === "zulip") && (
          <label>
            {tokenLabels[channelType]}
            {channelType === "ntfy" && " (optional)"}
            <input
              required={channelType !== "ntfy"}
              type="password"
              autoComplete="new-password"
              value={token}
              disabled={createChannel.isPending}
              onChange={(event) => setToken(event.target.value)}
            />
          </label>
        )}
        {channelType === "zulip" &&
          (
            Object.keys(zulipFieldLabels) as (keyof typeof zulipFieldLabels)[]
          ).map((field) => (
            <label key={field}>
              {zulipFieldLabels[field]}
              <input
                required
                type={field === "bot_email" ? "email" : "text"}
                maxLength={field === "bot_email" ? 254 : 60}
                value={zulip[field]}
                disabled={createChannel.isPending}
                onChange={(event) =>
                  setZulip((current) => ({
                    ...current,
                    [field]: event.target.value,
                  }))
                }
              />
            </label>
          ))}
        {channelType === "zulip" && (
          <p className="section-description">
            Use a bot with permission to post to the selected channel. All
            alerts use the configured topic.
          </p>
        )}
        {channelError && !editDraft && (
          <p className="alert error" role="alert">
            {channelError}
          </p>
        )}
        <button className="button primary" disabled={createChannel.isPending}>
          {createChannel.isPending ? "Saving…" : "Add channel"}
        </button>
      </form>
      {channels.isLoading && (
        <LoadingSkeleton label="Loading delivery channels" inline />
      )}
      {channels.isError && !channels.data && (
        <p className="alert error" role="alert">
          Channels could not be loaded.{" "}
          <button
            className="button ghost"
            onClick={() => void channels.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      {channels.data && channels.data.length > 0 && (
        <div className="channel-list-section">
          <h3>
            Your channels <span>({channels.data.length})</span>
          </h3>
          <ul className="plain-list channel-list">
            {channels.data.map((channel) => {
              const usageCount = ruleUsage.get(channel.id) ?? 0;
              return (
                <li key={channel.id}>
                  <div className="channel-row">
                    <span className="channel-identity">
                      <strong>{channel.name}</strong>
                      <span className="channel-summary">
                        {channelLabels[channel.channel_type]}
                        {rules.data && usageCount > 0 && (
                          <>
                            {" · "}
                            <button
                              className="channel-rule-count"
                              type="button"
                              aria-pressed={highlightedChannelId === channel.id}
                              onClick={() => onHighlightRules(channel.id)}
                            >
                              {usageCount} alert rule
                              {usageCount === 1 ? "" : "s"}
                            </button>
                          </>
                        )}
                        {rules.data && usageCount === 0 && " · 0 alert rules"}
                      </span>
                    </span>
                    <StatusBadge
                      value={channel.enabled ? "active" : "disabled"}
                      tone={channel.enabled ? "operational" : "neutral"}
                    />
                    {editDraft?.id !== channel.id && (
                      <details className="channel-menu">
                        <summary aria-label={`Actions for ${channel.name}`}>
                          ⋮
                        </summary>
                        <div
                          className="channel-menu-actions"
                          role="menu"
                          aria-label={`${channel.name} actions`}
                        >
                          <button
                            className="channel-menu-item"
                            role="menuitem"
                            onClick={() => editChannel(channel)}
                          >
                            Edit channel
                          </button>
                          <button
                            className="channel-menu-item"
                            role="menuitem"
                            disabled={updateChannel.isPending}
                            onClick={() => {
                              if (channel.enabled) {
                                updateChannel.reset();
                                setChannelToDisable(channel);
                              } else
                                updateChannel.mutate({
                                  id: channel.id,
                                  name: channel.name,
                                  enabled: true,
                                });
                            }}
                          >
                            {updateChannel.isPending &&
                              updateChannel.variables?.id === channel.id &&
                              "Updating…"}
                            {(!updateChannel.isPending ||
                              updateChannel.variables?.id !== channel.id) &&
                              (channel.enabled
                                ? "Disable channel"
                                : "Enable channel")}
                          </button>
                          <button
                            className="channel-menu-item"
                            role="menuitem"
                            onClick={() => testChannel.mutate(channel.id)}
                            disabled={testChannel.isPending || !channel.enabled}
                          >
                            {testChannel.isPending &&
                            testChannel.variables === channel.id
                              ? "Sending…"
                              : "Send test"}
                          </button>
                          <button
                            className="channel-menu-item danger"
                            role="menuitem"
                            onClick={() => {
                              deleteChannel.reset();
                              setChannelToRemove(channel);
                            }}
                          >
                            Remove channel
                          </button>
                        </div>
                      </details>
                    )}
                  </div>
                  {editDraft?.id === channel.id ? (
                    <div className="channel-edit-form">
                      <label>
                        Delivery channel name
                        <input
                          value={editDraft.name}
                          disabled={updateChannel.isPending}
                          onChange={(event) =>
                            updateEditDraft({ name: event.target.value })
                          }
                        />
                      </label>
                      {channel.channel_type === "webhook" && (
                        <SigningSecretField
                          value={editDraft.signingSecret}
                          disabled={updateChannel.isPending}
                          onChange={(value) =>
                            updateEditDraft({ signingSecret: value })
                          }
                          placeholder="Leave blank to keep signing secret"
                        />
                      )}
                      {(channel.channel_type === "gotify" ||
                        channel.channel_type === "ntfy" ||
                        channel.channel_type === "zulip") && (
                        <label>
                          {tokenLabels[channel.channel_type]}
                          <input
                            type="password"
                            autoComplete="new-password"
                            placeholder="Leave blank to keep existing token/key"
                            disabled={updateChannel.isPending}
                            value={editDraft.token}
                            onChange={(event) =>
                              updateEditDraft({ token: event.target.value })
                            }
                          />
                        </label>
                      )}
                      {channel.channel_type === "zulip" &&
                        (
                          Object.keys(
                            zulipFieldLabels,
                          ) as (keyof typeof zulipFieldLabels)[]
                        ).map((field) => (
                          <label key={field}>
                            {zulipFieldLabels[field]}
                            <input
                              type={field === "bot_email" ? "email" : "text"}
                              maxLength={field === "bot_email" ? 254 : 60}
                              placeholder="Leave blank to keep existing value"
                              value={editDraft[field]}
                              disabled={updateChannel.isPending}
                              onChange={(event) =>
                                updateEditDraft({ [field]: event.target.value })
                              }
                            />
                          </label>
                        ))}
                      <label>
                        Destination URL
                        <input
                          type="url"
                          placeholder="Leave blank to keep destination"
                          disabled={updateChannel.isPending}
                          value={editDraft.target}
                          onChange={(event) =>
                            updateEditDraft({ target: event.target.value })
                          }
                        />
                      </label>
                      {channelError && (
                        <p className="alert error" role="alert">
                          {channelError}
                        </p>
                      )}
                      <div className="channel-edit-actions">
                        <button
                          className="button primary"
                          type="button"
                          disabled={updateChannel.isPending}
                          onClick={() =>
                            updateChannel.mutate({
                              id: channel.id,
                              name: editDraft.name,
                              target: editDraft.target || undefined,
                              signingSecret:
                                editDraft.signingSecret || undefined,
                              token: editDraft.token || undefined,
                              bot_email: editDraft.bot_email || undefined,
                              stream: editDraft.stream || undefined,
                              topic: editDraft.topic || undefined,
                            })
                          }
                        >
                          {updateChannel.isPending ? "Saving…" : "Save"}
                        </button>
                        <button
                          className="button ghost"
                          type="button"
                          onClick={cancelChannelEdit}
                          disabled={updateChannel.isPending}
                        >
                          Cancel
                        </button>
                      </div>
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {channels.data && channels.data.length === 0 && (
        <EmptyState
          title="No delivery channels"
          description="Add a webhook destination above to begin delivering notifications."
          inline
        />
      )}
      {channelToRemove && (
        <ConfirmDialog
          title={`Remove delivery channel “${channelToRemove.name}”?`}
          message="Remove this channel from its alert rules before deleting it."
          confirmLabel="Remove channel"
          pending={deleteChannel.isPending}
          error={deleteChannel.error?.message}
          onConfirm={() => deleteChannel.mutate(channelToRemove.id)}
          onClose={() => setChannelToRemove(undefined)}
        />
      )}
      {channelToDisable && (
        <ConfirmDialog
          title={`Disable delivery channel “${channelToDisable.name}”?`}
          message="Pending, retrying, and held deliveries will be cancelled. Alert-rule associations remain in place, and re-enabling applies only to new events. A delivery already in flight may still complete."
          confirmLabel="Disable channel"
          pending={updateChannel.isPending}
          error={updateChannel.error?.message}
          onConfirm={() =>
            updateChannel.mutate({
              id: channelToDisable.id,
              name: channelToDisable.name,
              enabled: false,
            })
          }
          onClose={() => setChannelToDisable(undefined)}
        />
      )}
    </section>
  );
}

function SigningSecretField({
  value,
  onChange,
  placeholder,
  disabled,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  disabled: boolean;
}) {
  const inputId = useId();

  return (
    <div className="signing-secret-field">
      <span className="field-help-label">
        <label htmlFor={inputId}>HMAC signing secret (optional)</label>
        <FieldHelp
          label="HMAC signing secret"
          className="signing-secret-help"
          help={
            <>
              <span>
                Add a shared secret (32–4,096 characters) so your receiver can
                verify that a webhook came from StatusDeck. The secret is never
                sent.
              </span>
              <span>Signed requests include these headers:</span>
              <code>{`Content-Type: application/json
X-StatusDeck-Event-Id: <event UUID>
X-StatusDeck-Timestamp: <Unix timestamp in seconds>
X-StatusDeck-Signature: sha256=<hex digest>`}</code>
              <span>The signature is calculated as:</span>
              <code>
                {'HMAC-SHA256(secret, timestamp + "." + raw JSON body)'}
              </code>
              <span>
                Verify with the same secret and original body bytes using a
                constant-time comparison. Reject stale timestamps to limit
                replay; use the event ID to detect duplicate deliveries.
              </span>
            </>
          }
        />
      </span>
      <input
        id={inputId}
        minLength={32}
        type="password"
        autoComplete="new-password"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
      />
    </div>
  );
}
