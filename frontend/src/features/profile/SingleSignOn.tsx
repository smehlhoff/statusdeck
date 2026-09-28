import { CopyButton } from "../../components/CopyButton";
import { useToast } from "../../components/toastContext";
import { FieldHelp } from "../../components/FieldHelp";
import { LoadingDots } from "../../components/LoadingDots";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { ApiFailure, api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import { SsoSettings, type SsoConfiguration } from "./SsoSettings";

type Identity = { issuer: string; subject: string; needs_relink?: boolean };
type SsoStatus = {
  revision: number;
  callback_url: string;
  configuration: SsoConfiguration;
  enabled: boolean;
  available: boolean;
  label: string;
  authentication_method: "local" | "oidc";
  linked: Identity | null;
  pending: Identity | null;
};

export function SingleSignOn() {
  const client = useQueryClient();
  const { notify } = useToast();
  const [params] = useSearchParams();
  const [password, setPassword] = useState("");
  const status = useQuery({
    queryKey: ["profile-oidc"],
    queryFn: () => api<SsoStatus>("/api/v1/profile/oidc"),
    staleTime: 0,
  });
  const change = useMutation({
    mutationFn: async (action: "link" | "confirm" | "disconnect") => {
      if (action === "link") {
        const result = await api<{ authorization_url: string }>(
          "/api/v1/profile/oidc/link",
          {
            method: "POST",
            body: JSON.stringify({ current_password: password }),
          },
        );
        setPassword("");
        window.location.assign(result.authorization_url);
      } else {
        await api<void>(
          `/api/v1/profile/oidc/link${action === "confirm" ? "/confirm" : ""}`,
          {
            method: action === "confirm" ? "POST" : "DELETE",
            ...(action === "disconnect"
              ? { body: JSON.stringify({ current_password: password }) }
              : {}),
          },
        );
        notify(
          action === "confirm"
            ? "Account linked successfully. You can now sign in with SSO."
            : "Account disconnected successfully.",
        );
        setPassword("");
        await Promise.all([
          client.invalidateQueries({ queryKey: ["profile-oidc"] }),
          client.invalidateQueries({ queryKey: queryKeys.profileSessions }),
          client.invalidateQueries({ queryKey: queryKeys.securityActivity }),
        ]);
      }
    },
  });

  const info = status.data;
  const linked = info?.linked;
  const pending = info?.pending;
  const actionLabel = info?.linked
    ? "Disconnect account"
    : "Link with SSO provider";
  return (
    <section className="card profile-section" aria-labelledby="profile-sso">
      <header className="profile-section-heading">
        <h2 id="profile-sso">Single sign-on</h2>
        <Link
          className="button ghost sso-instructions-button"
          to="/profile/sso-instructions"
        >
          Setup instructions
        </Link>
      </header>
      {params.has("oidc") && (
        <p className="alert error" role="alert">
          The sign-in attempt failed or expired. Try linking your account again.
        </p>
      )}
      {status.isPending && (
        <p>
          <LoadingDots label="Loading single sign-on" />
        </p>
      )}
      {status.isError && (
        <p className="alert error" role="alert">
          Single sign-on settings could not be loaded.{" "}
          <button
            className="button ghost"
            onClick={() => void status.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      {change.isError && (
        <p className="alert error" role="alert">
          {change.error instanceof ApiFailure
            ? change.error.message
            : "The request failed. Try again."}
        </p>
      )}
      {info && (
        <>
          {pending && info.authentication_method === "local" && (
            <p className="alert error" role="alert">
              You&apos;ve signed in with your SSO provider. Confirm below to
              finish linking this account to StatusDeck.
            </p>
          )}
          {info.authentication_method === "local" && (
            <SsoSettings
              key={info.revision}
              configuration={info.configuration}
              revision={info.revision}
              callbackUrl={info.callback_url}
            />
          )}
          {info.enabled && !info.available && (
            <p className="profile-help sso-status" role="status">
              Provider unavailable. Check your settings and try again.
            </p>
          )}
          {linked && (
            <section
              className="sso-identity"
              aria-labelledby="sso-linked-account"
            >
              <h3 id="sso-linked-account">Linked account</h3>
              <dl className="sso-identity-details sso-endpoint">
                <div>
                  <dt className="field-help-label sso-help-label">
                    Provider
                    <FieldHelp
                      label="Linked provider"
                      help="The issuer that verified this account’s identity."
                    />
                  </dt>
                  <dd>
                    <pre>
                      <code>{linked.issuer}</code>
                    </pre>
                    <CopyButton
                      className="button ghost compact"
                      ariaLabel="Copy provider"
                      value={linked.issuer}
                      errorMessage="Could not copy the account information."
                    />
                  </dd>
                </div>
                <div>
                  <dt className="field-help-label sso-help-label">
                    Account ID
                    <FieldHelp
                      label="Account ID"
                      help="Your provider’s unique subject identifier. StatusDeck uses this with the issuer to identify your account; it may not look like your username or email."
                    />
                  </dt>
                  <dd>
                    <pre>
                      <code>{linked.subject}</code>
                    </pre>
                    <CopyButton
                      className="button ghost compact"
                      ariaLabel="Copy account id"
                      value={linked.subject}
                      errorMessage="Could not copy the account information."
                    />
                  </dd>
                </div>
              </dl>
              {linked.needs_relink && (
                <p className="profile-help" role="status">
                  Provider settings changed. Disconnect and relink your account.
                </p>
              )}
            </section>
          )}
          {info.authentication_method !== "local" ? (
            <p className="profile-help">
              Sign in with your local email and password to manage SSO.
            </p>
          ) : (
            <>
              {pending && (
                <section
                  className="sso-identity sso-confirmation"
                  aria-labelledby="sso-confirm-account"
                >
                  <h3 id="sso-confirm-account">Confirm your account</h3>
                  <dl className="sso-identity-details sso-endpoint">
                    <div>
                      <dt className="field-help-label sso-help-label">
                        Provider
                        <FieldHelp
                          label="Linked provider"
                          help="The issuer that verified this account’s identity."
                        />
                      </dt>
                      <dd>
                        <pre>
                          <code>{pending.issuer}</code>
                        </pre>
                        <CopyButton
                          className="button ghost compact"
                          ariaLabel="Copy provider"
                          value={pending.issuer}
                          errorMessage="Could not copy the account information."
                        />
                      </dd>
                    </div>
                    <div>
                      <dt className="field-help-label sso-help-label">
                        Account ID
                        <FieldHelp
                          label="Account ID"
                          help="Your provider’s unique subject identifier. StatusDeck uses this with the issuer to identify your account; it may not look like your username or email."
                        />
                      </dt>
                      <dd>
                        <pre>
                          <code>{pending.subject}</code>
                        </pre>
                        <CopyButton
                          className="button ghost compact"
                          ariaLabel="Copy account id"
                          value={pending.subject}
                          errorMessage="Could not copy the account information."
                        />
                      </dd>
                    </div>
                  </dl>
                  <button
                    className="button primary"
                    disabled={change.isPending}
                    onClick={() => change.mutate("confirm")}
                  >
                    {change.isPending ? "Confirming…" : "Confirm account"}
                  </button>
                  <p className="profile-help">
                    Wrong account? Switch accounts at your provider and link
                    again. This request expires in ten minutes.
                  </p>
                </section>
              )}
              {(linked || info.available) && (
                <form
                  className="sso-account"
                  onSubmit={(event) => {
                    event.preventDefault();
                    if (!change.isPending)
                      change.mutate(linked ? "disconnect" : "link");
                  }}
                >
                  {!linked && <h3>Link your account</h3>}
                  <label>
                    Confirm with your local password
                    <input
                      type="password"
                      autoComplete="current-password"
                      required
                      maxLength={1024}
                      value={password}
                      onChange={(event) => setPassword(event.target.value)}
                    />
                  </label>
                  {linked && (
                    <p className="profile-help">
                      Disconnecting signs out all SSO sessions.
                    </p>
                  )}
                  <button
                    className={`button ${linked ? "danger" : "primary"}`}
                    disabled={change.isPending || !password}
                  >
                    {change.isPending ? "Working…" : actionLabel}
                  </button>
                </form>
              )}
            </>
          )}
        </>
      )}
    </section>
  );
}
