import { FieldHelp } from "../../components/FieldHelp";
import { CopyButton } from "../../components/CopyButton";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useId, useState } from "react";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { useToast } from "../../components/toastContext";

export interface SsoConfiguration {
  enabled: boolean;
  label: string;
  issuer_url: string;
  discovery_url: string;
  client_id: string;
  has_client_secret: boolean;
  token_auth_method: "client_secret_basic" | "client_secret_post";
  allowed_endpoint_origins: string[];
  session_max_age_seconds: number;
  ca_certificate_pem: string;
}

export function SsoSettings({
  configuration,
  revision,
  callbackUrl,
}: {
  configuration: SsoConfiguration;
  revision: number;
  callbackUrl: string;
}) {
  const fieldId = useId();
  const logoutUrl = callbackUrl.replace(/\/callback$/, "/backchannel-logout");
  const { has_client_secret: hasSecret, ...saved } = configuration;
  const [draft, setDraft] = useState({ ...saved, client_secret: "" });
  const [origins, setOrigins] = useState(
    saved.allowed_endpoint_origins.join("\n"),
  );
  const [password, setPassword] = useState("");
  const [confirmClear, setConfirmClear] = useState(false);
  const showActions = draft.enabled || saved.enabled || revision > 0;
  const client = useQueryClient();
  const { notify } = useToast();
  const save = useMutation({
    mutationFn: (action: "save" | "clear") =>
      api<void>("/api/v1/profile/oidc", {
        method: action === "clear" ? "DELETE" : "PUT",
        body: JSON.stringify({
          revision,
          current_password: password,
          ...(action === "save"
            ? {
                configuration: {
                  ...draft,
                  allowed_endpoint_origins: origins
                    .split(/[\n,]/)
                    .map((value) => value.trim())
                    .filter(Boolean),
                },
              }
            : {}),
        }),
      }),
    onSuccess: async (_, action) => {
      setConfirmClear(false);
      setPassword("");
      setDraft((current) => ({ ...current, client_secret: "" }));
      notify(
        action === "clear"
          ? "Single sign-on settings cleared."
          : "Single sign-on settings saved.",
      );
      await Promise.all([
        client.invalidateQueries({ queryKey: ["profile-oidc"] }),
        client.invalidateQueries({ queryKey: ["auth-methods"] }),
        client.invalidateQueries({ queryKey: queryKeys.profileSessions }),
        client.invalidateQueries({ queryKey: queryKeys.securityActivity }),
      ]);
    },
  });

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (!save.isPending) save.mutate("save");
      }}
    >
      <fieldset disabled={save.isPending}>
        <div className="field-help-label sso-help-label">
          <label className="sso-toggle">
            <input
              type="checkbox"
              checked={draft.enabled}
              onChange={(event) =>
                setDraft({ ...draft, enabled: event.target.checked })
              }
            />
            Enable single sign-on (OIDC)
          </label>
          <FieldHelp
            label="Enable single sign-on"
            help="Changes apply when saved. Disabling SSO ends SSO sessions but retains your settings and linked account. Local sign-in stays available."
          />
        </div>
        <p className="profile-help">
          Sign in with your identity provider. Email and password remain
          available as backup.
        </p>
        {draft.enabled && (
          <>
            <div className="profile-field-grid">
              <div className="sso-field">
                <FieldLabel
                  id={`${fieldId}-provider`}
                  label="Provider name"
                  help="The name shown on the sign-in button, such as Authentik."
                />
                <input
                  id={`${fieldId}-provider`}
                  required
                  maxLength={100}
                  value={draft.label}
                  onChange={(event) =>
                    setDraft({ ...draft, label: event.target.value })
                  }
                />
              </div>
              <div className="sso-field">
                <FieldLabel
                  id={`${fieldId}-issuer`}
                  label="Issuer URL"
                  help="Copy the issuer from your provider’s discovery document, including its path and trailing slash."
                />
                <input
                  id={`${fieldId}-issuer`}
                  type="url"
                  required
                  maxLength={2048}
                  value={draft.issuer_url}
                  onChange={(event) =>
                    setDraft({ ...draft, issuer_url: event.target.value })
                  }
                />
              </div>
              <div className="sso-field">
                <FieldLabel
                  id={`${fieldId}-client-id`}
                  label="Client ID"
                  help="The application identifier from your provider."
                />
                <input
                  id={`${fieldId}-client-id`}
                  required
                  maxLength={1024}
                  autoComplete="off"
                  value={draft.client_id}
                  onChange={(event) =>
                    setDraft({ ...draft, client_id: event.target.value })
                  }
                />
              </div>
              <div className="sso-field">
                <FieldLabel
                  id={`${fieldId}-secret`}
                  label="Client secret"
                  help="The application secret from your provider. Stored encrypted and never displayed after saving. Leave blank to keep the saved secret."
                />
                <input
                  id={`${fieldId}-secret`}
                  type="password"
                  required={!hasSecret}
                  maxLength={8192}
                  autoComplete="new-password"
                  value={draft.client_secret}
                  onChange={(event) =>
                    setDraft({ ...draft, client_secret: event.target.value })
                  }
                />
              </div>
            </div>
            <dl className="sso-identity-details sso-endpoint">
              <div>
                <dt className="field-help-label sso-help-label">
                  Redirect URI
                  <FieldHelp
                    label="Redirect URI"
                    help="Copy this exact URL into your provider’s allowed redirect URIs."
                  />
                </dt>
                <dd>
                  <pre>
                    <code>{callbackUrl}</code>
                  </pre>
                  <CopyButton
                    className="button ghost compact"
                    ariaLabel="Copy redirect URI"
                    value={callbackUrl}
                    errorMessage="Could not copy redirect uri."
                  />
                </dd>
              </div>
            </dl>
            <div className="sso-field">
              <FieldLabel
                id={`${fieldId}-discovery`}
                label="Discovery URL (optional)"
                help="Leave blank to discover endpoints from the issuer. Set this only if your provider uses a different discovery document URL."
              />
              <input
                id={`${fieldId}-discovery`}
                type="url"
                maxLength={2048}
                value={draft.discovery_url}
                onChange={(event) =>
                  setDraft({ ...draft, discovery_url: event.target.value })
                }
              />
            </div>
            <div className="sso-field">
              <FieldLabel
                id={`${fieldId}-auth-method`}
                label="Client authentication method"
                help="How StatusDeck sends client credentials to the token endpoint."
              />
              <select
                id={`${fieldId}-auth-method`}
                value={draft.token_auth_method}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    token_auth_method: event.target
                      .value as SsoConfiguration["token_auth_method"],
                  })
                }
              >
                <option value="client_secret_basic">
                  HTTP header (client_secret_basic)
                </option>
                <option value="client_secret_post">
                  Request body (client_secret_post)
                </option>
              </select>
            </div>
            <div className="sso-field">
              <FieldLabel
                id={`${fieldId}-origins`}
                label="Additional trusted origins (optional)"
                help="Only needed when provider endpoints use another origin. Enter one HTTPS origin per line, such as https://login.example.com. The issuer origin is already trusted."
              />
              <textarea
                id={`${fieldId}-origins`}
                rows={3}
                maxLength={32784}
                value={origins}
                onChange={(event) => setOrigins(event.target.value)}
              />
            </div>
            <div className="sso-field">
              <FieldLabel
                id={`${fieldId}-lifetime`}
                label="Session lifetime (seconds)"
                help="How long an SSO session lasts: 1–28,800 seconds (up to eight hours). Reducing this also shortens existing SSO sessions."
              />
              <input
                id={`${fieldId}-lifetime`}
                type="number"
                min={1}
                max={28800}
                step={1}
                required
                value={draft.session_max_age_seconds}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    session_max_age_seconds: Number(event.target.value),
                  })
                }
              />
            </div>
            <div className="sso-field">
              <FieldLabel
                id={`${fieldId}-certificate`}
                label="Private CA certificate (optional, PEM)"
                help="For a provider using a private certificate authority, paste its CA certificate in PEM format. Leave blank for publicly trusted HTTPS. Never paste a private key."
              />
              <textarea
                id={`${fieldId}-certificate`}
                rows={5}
                maxLength={65536}
                value={draft.ca_certificate_pem}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    ca_certificate_pem: event.target.value,
                  })
                }
              />
            </div>
            <dl className="sso-identity-details sso-endpoint">
              <div>
                <dt className="field-help-label sso-help-label">
                  Back-channel logout URL
                  <FieldHelp
                    label="Back-channel logout URL"
                    help="Optional: set this as your provider’s back-channel Logout URI so it can end StatusDeck sessions. This is not a redirect URI or the provider’s end-session URL."
                  />
                </dt>
                <dd>
                  <pre>
                    <code>{logoutUrl}</code>
                  </pre>
                  <CopyButton
                    className="button ghost compact"
                    ariaLabel="Copy back-channel logout URL"
                    value={logoutUrl}
                    errorMessage="Could not copy back-channel logout url."
                  />
                </dd>
              </div>
            </dl>
          </>
        )}
        {showActions && (
          <>
            {saved.enabled && (
              <p className="profile-help">
                {draft.enabled
                  ? "Changing the issuer or client ID requires relinking your account."
                  : "Saving will disable SSO and sign out SSO sessions."}
              </p>
            )}
            <div className="sso-field">
              <FieldLabel
                id={`${fieldId}-password`}
                label="Confirm with your local password"
                help="Enter the password you use to sign in directly to StatusDeck to save or clear these settings."
              />
              <input
                id={`${fieldId}-password`}
                type="password"
                autoComplete="current-password"
                required
                maxLength={1024}
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
            </div>
          </>
        )}
      </fieldset>
      {save.isError && !confirmClear && (
        <p className="alert error" role="alert">
          {save.error.message}
        </p>
      )}
      {showActions && (
        <div className="card-actions">
          <button
            className="button primary"
            disabled={save.isPending || !password}
          >
            {save.isPending && save.variables === "save"
              ? "Saving…"
              : "Save changes"}
          </button>
          <button
            type="button"
            className="button ghost"
            disabled={save.isPending || !password}
            onClick={() => {
              save.reset();
              setConfirmClear(true);
            }}
          >
            Clear settings
          </button>
        </div>
      )}
      {confirmClear && (
        <ConfirmDialog
          title="Clear single sign-on settings?"
          message="This removes the saved provider settings, client secret, and linked account, and signs out all SSO sessions. Local sign-in remains available."
          confirmLabel="Clear settings"
          pending={save.isPending}
          error={save.isError ? save.error.message : undefined}
          onConfirm={() => {
            if (!save.isPending) save.mutate("clear");
          }}
          onClose={() => {
            setConfirmClear(false);
            save.reset();
          }}
        />
      )}
    </form>
  );
}

function FieldLabel({
  id,
  label,
  help,
}: {
  id: string;
  label: string;
  help: string;
}) {
  return (
    <span className="field-help-label sso-help-label">
      <label htmlFor={id}>{label}</label>
      <FieldHelp label={label} help={help} />
    </span>
  );
}
