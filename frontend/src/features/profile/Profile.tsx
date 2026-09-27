import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import {
  Link,
  useBeforeUnload,
  useBlocker,
  useSearchParams,
} from "react-router-dom";
import { ApiFailure, api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { DisplayPreferences, ProfileSession, User } from "../../api/types";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { UserAvatar } from "../../components/UserAvatar";
import { useToast } from "../../components/toastContext";
import { formatDateTime } from "../../utils/display";
import { useProfile } from "./profileContext";
import { Appearance } from "./Appearance";
import { SecurityActivity } from "./SecurityActivity";
import { SingleSignOn } from "./SingleSignOn";
import { TimeZoneSelect } from "./TimeZoneSelect";

function message(error: unknown): string {
  return error instanceof ApiFailure
    ? error.message
    : "The change could not be saved. Check the connection and try again.";
}

function deviceName(userAgent: string | null): string {
  if (!userAgent) return "Existing session";
  const browsers: [RegExp, string][] = [
    [/Edg\//, "Edge"],
    [/Firefox\//, "Firefox"],
    [/Chrome\//, "Chrome"],
    [/Safari\//, "Safari"],
  ];
  const systems: [RegExp, string][] = [
    [/Android/, "Android"],
    [/iPhone|iPad/, "iOS"],
    [/Windows/, "Windows"],
    [/Macintosh/, "macOS"],
    [/Linux/, "Linux"],
  ];
  const browser =
    browsers.find(([pattern]) => pattern.test(userAgent))?.[1] ?? "Browser";
  const system =
    systems.find(([pattern]) => pattern.test(userAgent))?.[1] ??
    "Unknown device";
  return `${browser} · ${system}`;
}

const PROFILE_SECTIONS = [
  ["identity", "Identity"],
  ["preferences", "Display preferences"],
  ["appearance", "Appearance"],
  ["email", "Login email"],
  ["password", "Password"],
  ["sso", "Single sign-on"],
  ["security", "Security activity"],
  ["sessions", "Session history"],
] as const;

const SESSION_STATUS_LABELS = {
  active: "Active",
  expired: "Expired",
  ended: "Signed out",
};

export function Profile() {
  const profile = useProfile();
  const [searchParams] = useSearchParams();
  const selectedSection = searchParams.get("section");
  const section =
    PROFILE_SECTIONS.find(([key]) => key === selectedSection)?.[0] ??
    "identity";
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const [displayName, setDisplayName] = useState(profile.display_name);
  const [preferences, setPreferences] = useState(profile.preferences);
  const [email, setEmail] = useState(profile.email);
  const [emailPassword, setEmailPassword] = useState("");
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [nameError, setNameError] = useState("");
  const [preferencesError, setPreferencesError] = useState("");
  const [emailError, setEmailError] = useState("");
  const [passwordError, setPasswordError] = useState("");
  const [revokeTarget, setRevokeTarget] = useState<
    ProfileSession | "others" | null
  >(null);
  const [sessionError, setSessionError] = useState("");
  const preferencesDirty =
    preferences.time_zone !== profile.preferences.time_zone ||
    preferences.date_format !== profile.preferences.date_format ||
    preferences.time_format !== profile.preferences.time_format ||
    preferences.timestamp_format !== profile.preferences.timestamp_format ||
    preferences.landing_page !== profile.preferences.landing_page ||
    preferences.refresh_interval_ms !== profile.preferences.refresh_interval_ms;
  const blocker = useBlocker(preferencesDirty);

  useBeforeUnload(
    useCallback(
      (event) => {
        if (!preferencesDirty) return;
        event.preventDefault();
        event.returnValue = "";
      },
      [preferencesDirty],
    ),
  );

  async function acceptProfile(updated: User) {
    await queryClient.cancelQueries({ queryKey: queryKeys.session });
    queryClient.setQueryData(queryKeys.session, updated);
    await queryClient.invalidateQueries({ queryKey: ["incident-comments"] });
  }

  const saveName = useMutation({
    mutationFn: () =>
      api<User>("/api/v1/profile", {
        method: "PATCH",
        body: JSON.stringify({ display_name: displayName }),
      }),
    onSuccess: async (updated) => {
      setDisplayName(updated.display_name);
      setNameError("");
      notify("Profile updated.");
      await acceptProfile(updated);
    },
    onError: (error) => setNameError(message(error)),
  });
  const savePreferences = useMutation({
    mutationFn: () =>
      api<User>("/api/v1/profile", {
        method: "PATCH",
        body: JSON.stringify({
          preferences: {
            time_zone: preferences.time_zone,
            date_format: preferences.date_format,
            time_format: preferences.time_format,
            timestamp_format: preferences.timestamp_format,
            landing_page: preferences.landing_page,
            refresh_interval_ms: preferences.refresh_interval_ms,
          },
        }),
      }),
    onSuccess: async (updated) => {
      setPreferences(updated.preferences);
      setPreferencesError("");
      notify("Display preferences saved.");
      await acceptProfile(updated);
    },
    onError: (error) => setPreferencesError(message(error)),
  });
  const changeEmail = useMutation({
    mutationFn: () =>
      api<User>("/api/v1/profile/email", {
        method: "PATCH",
        body: JSON.stringify({ email, current_password: emailPassword }),
      }),
    onSuccess: async (updated) => {
      setEmail(updated.email);
      setEmailPassword("");
      setEmailError("");
      notify("Email changed. Other sessions have been signed out.");
      await acceptProfile(updated);
      await queryClient.invalidateQueries({
        queryKey: queryKeys.profileSessions,
      });
    },
    onError: (error) => setEmailError(message(error)),
  });
  const changePassword = useMutation({
    mutationFn: () =>
      api<void>("/api/v1/profile/password", {
        method: "PATCH",
        body: JSON.stringify({
          current_password: currentPassword,
          new_password: newPassword,
        }),
      }),
    onSuccess: async () => {
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      setPasswordError("");
      notify("Password changed. Other sessions have been signed out.");
      await queryClient.invalidateQueries({
        queryKey: queryKeys.profileSessions,
      });
    },
    onError: (error) => setPasswordError(message(error)),
  });
  const sessions = useQuery({
    queryKey: queryKeys.profileSessions,
    enabled: section === "sessions",
    queryFn: ({ signal }) =>
      api<ProfileSession[]>("/api/v1/profile/sessions", { signal }),
    refetchInterval: 30_000,
    refetchIntervalInBackground: false,
  });
  const revoke = useMutation({
    mutationFn: (target: ProfileSession | "others") =>
      api<void>(
        `/api/v1/profile/sessions${target === "others" ? "" : `/${target.id}`}`,
        { method: "DELETE" },
      ),
    onSuccess: async () => {
      setRevokeTarget(null);
      setSessionError("");
      notify("Sessions signed out.");
      await queryClient.invalidateQueries({
        queryKey: queryKeys.profileSessions,
      });
    },
    onError: (error) => setSessionError(message(error)),
  });
  const busy =
    saveName.isPending ||
    savePreferences.isPending ||
    changeEmail.isPending ||
    changePassword.isPending ||
    revoke.isPending;

  function preference<K extends keyof DisplayPreferences>(
    key: K,
    value: DisplayPreferences[K],
  ) {
    setPreferences((current) => ({ ...current, [key]: value }));
  }

  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Profile</h1>
          <p className="muted">
            Manage your identity, preferences, and account security.
          </p>
        </div>
      </div>
      <div className="profile-layout">
        <nav className="profile-nav" aria-label="Profile sections">
          {PROFILE_SECTIONS.map(([key, label]) => (
            <Link
              key={key}
              to={`?section=${key}`}
              aria-current={section === key ? "page" : undefined}
            >
              {label}
            </Link>
          ))}
        </nav>
        <div className="profile-content">
          {section === "sso" && <SingleSignOn />}
          {section === "identity" && (
            <section
              className="card profile-section"
              aria-labelledby="profile-identity"
            >
              <header className="profile-section-heading">
                <h2 id="profile-identity">Identity</h2>
              </header>
              <div className="profile-identity">
                <UserAvatar name={displayName} large />
                <div>
                  <strong>{profile.display_name}</strong>
                  <p className="muted">Administrator</p>
                </div>
              </div>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  if (!busy) {
                    setNameError("");
                    saveName.mutate();
                  }
                }}
              >
                <label>
                  Display name
                  <input
                    value={displayName}
                    onChange={(event) => setDisplayName(event.target.value)}
                    autoComplete="nickname"
                    required
                    maxLength={100}
                    disabled={saveName.isPending}
                  />
                </label>
                {nameError && (
                  <p className="alert error" role="alert">
                    {nameError}
                  </p>
                )}
                <button
                  className="button primary"
                  disabled={busy || !displayName.trim()}
                >
                  {saveName.isPending ? "Saving…" : "Save profile"}
                </button>
              </form>
            </section>
          )}
          {section === "preferences" && (
            <section
              className="card profile-section"
              aria-labelledby="profile-preferences"
            >
              <header className="profile-section-heading">
                <h2 id="profile-preferences">Display preferences</h2>
              </header>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  if (busy) return;
                  setPreferencesError("");
                  if (preferences.time_zone !== "browser") {
                    try {
                      new Intl.DateTimeFormat(undefined, {
                        timeZone: preferences.time_zone,
                      });
                    } catch {
                      setPreferencesError(
                        "Choose a time zone supported by your browser.",
                      );
                      return;
                    }
                  }
                  savePreferences.mutate();
                }}
              >
                <fieldset disabled={savePreferences.isPending}>
                  <TimeZoneSelect
                    value={preferences.time_zone}
                    onChange={(zone) => preference("time_zone", zone)}
                  />
                  <p className="profile-help" id="profile-time-zone-help">
                    Alert quiet hours keep their own time zone.
                  </p>
                  <div className="profile-field-grid">
                    <label>
                      Date format
                      <select
                        value={preferences.date_format}
                        onChange={(event) =>
                          preference(
                            "date_format",
                            event.target
                              .value as DisplayPreferences["date_format"],
                          )
                        }
                      >
                        <option value="locale">Browser locale</option>
                        <option value="iso">YYYY-MM-DD</option>
                        <option value="day_first">DD/MM/YYYY</option>
                        <option value="month_first">MM/DD/YYYY</option>
                      </select>
                    </label>
                    <label>
                      Time format
                      <select
                        value={preferences.time_format}
                        onChange={(event) =>
                          preference(
                            "time_format",
                            event.target
                              .value as DisplayPreferences["time_format"],
                          )
                        }
                      >
                        <option value="locale">Browser locale</option>
                        <option value="twelve_hour">12-hour</option>
                        <option value="twenty_four_hour">24-hour</option>
                      </select>
                    </label>
                    <label>
                      Timestamps
                      <select
                        value={preferences.timestamp_format}
                        onChange={(event) =>
                          preference(
                            "timestamp_format",
                            event.target
                              .value as DisplayPreferences["timestamp_format"],
                          )
                        }
                      >
                        <option value="relative">Relative</option>
                        <option value="absolute">Absolute</option>
                      </select>
                    </label>
                    <label>
                      Default landing page
                      <select
                        value={preferences.landing_page}
                        onChange={(event) =>
                          preference(
                            "landing_page",
                            event.target
                              .value as DisplayPreferences["landing_page"],
                          )
                        }
                      >
                        <option value="/">Overview</option>
                        <option value="/catalog">Providers</option>
                        <option value="/incidents">Incidents</option>
                        <option value="/analytics">Analytics</option>
                        <option value="/notifications">Notifications</option>
                        <option value="/system">System</option>
                        <option value="/profile">Profile</option>
                      </select>
                    </label>
                  </div>
                  <label>
                    Auto-refresh
                    <select
                      value={preferences.refresh_interval_ms}
                      onChange={(event) =>
                        preference(
                          "refresh_interval_ms",
                          Number(
                            event.target.value,
                          ) as DisplayPreferences["refresh_interval_ms"],
                        )
                      }
                    >
                      <option value={0}>Off</option>
                      <option value={5000}>Every 5 seconds</option>
                      <option value={15000}>Every 15 seconds</option>
                      <option value={60000}>Every minute</option>
                    </select>
                  </label>
                  <p className="profile-help">
                    Applies to Overview and Incidents across browsers and
                    devices. Detail and system pages retain their automatic
                    updates.
                  </p>
                </fieldset>
                <p className="profile-date-preview">
                  Date preview:{" "}
                  {formatDateTime("2026-09-12T17:30:00Z", preferences)}
                </p>
                {preferencesError && (
                  <p className="alert error" role="alert">
                    {preferencesError}
                  </p>
                )}
                <button className="button primary" disabled={busy}>
                  {savePreferences.isPending ? "Saving…" : "Save preferences"}
                </button>
              </form>
            </section>
          )}
          {section === "appearance" && <Appearance />}
          {section === "email" && (
            <section
              className="card profile-section"
              aria-labelledby="profile-email"
            >
              <header className="profile-section-heading">
                <h2 id="profile-email">Login email</h2>
                <p className="profile-help">
                  Changing your email signs out other sessions. This session
                  stays signed in.
                </p>
              </header>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  if (!busy) {
                    setEmailError("");
                    changeEmail.mutate();
                  }
                }}
              >
                <fieldset disabled={changeEmail.isPending}>
                  <label>
                    Email address
                    <input
                      type="email"
                      autoComplete="username"
                      value={email}
                      onChange={(event) => setEmail(event.target.value)}
                      required
                      maxLength={254}
                    />
                  </label>
                  <label>
                    Current password
                    <input
                      type="password"
                      autoComplete="current-password"
                      value={emailPassword}
                      onChange={(event) => setEmailPassword(event.target.value)}
                      required
                      maxLength={1024}
                    />
                  </label>
                </fieldset>
                {emailError && (
                  <p className="alert error" role="alert">
                    {emailError}
                  </p>
                )}
                <button
                  className="button primary"
                  disabled={
                    busy ||
                    !emailPassword ||
                    email.trim().toLowerCase() === profile.email
                  }
                >
                  {changeEmail.isPending ? "Changing…" : "Change email"}
                </button>
              </form>
            </section>
          )}
          {section === "password" && (
            <section
              className="card profile-section"
              aria-labelledby="profile-password"
            >
              <header className="profile-section-heading">
                <h2 id="profile-password">Password</h2>
                <p className="profile-help">
                  Changing your password signs out other sessions. This session
                  stays signed in.
                </p>
              </header>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  if (busy) return;
                  if (newPassword !== confirmPassword) {
                    setPasswordError("The new passwords do not match.");
                    return;
                  }
                  if (
                    Array.from(newPassword).length < 12 ||
                    Array.from(newPassword).length > 1024
                  ) {
                    setPasswordError("Use between 12 and 1,024 characters.");
                    return;
                  }
                  setPasswordError("");
                  changePassword.mutate();
                }}
              >
                <fieldset disabled={changePassword.isPending}>
                  <label>
                    Current password
                    <input
                      type="password"
                      autoComplete="current-password"
                      value={currentPassword}
                      onChange={(event) =>
                        setCurrentPassword(event.target.value)
                      }
                      required
                      maxLength={1024}
                    />
                  </label>
                  <label>
                    New password
                    <input
                      type="password"
                      autoComplete="new-password"
                      value={newPassword}
                      onChange={(event) => setNewPassword(event.target.value)}
                      required
                      maxLength={1024}
                    />
                  </label>
                  <label>
                    Confirm new password
                    <input
                      type="password"
                      autoComplete="new-password"
                      value={confirmPassword}
                      onChange={(event) =>
                        setConfirmPassword(event.target.value)
                      }
                      required
                      maxLength={1024}
                    />
                  </label>
                </fieldset>
                {passwordError && (
                  <p className="alert error" role="alert">
                    {passwordError}
                  </p>
                )}
                <button
                  className="button primary"
                  disabled={
                    busy || !currentPassword || !newPassword || !confirmPassword
                  }
                >
                  {changePassword.isPending ? "Changing…" : "Change password"}
                </button>
              </form>
            </section>
          )}
          {section === "security" && <SecurityActivity />}
          {section === "sessions" && (
            <section
              className="card profile-section profile-sessions"
              aria-labelledby="profile-sessions"
            >
              <header className="profile-section-heading">
                <div>
                  <h2 id="profile-sessions">Session history</h2>
                  <p className="profile-help">
                    Active sessions and up to 100 past sessions from the last 90
                    days. Sessions expire after 30 days or 7 days of inactivity.
                  </p>
                </div>
                <button
                  className="button ghost"
                  disabled={
                    busy ||
                    !sessions.data?.some(
                      (session) =>
                        !session.current && session.status === "active",
                    )
                  }
                  onClick={() => {
                    setSessionError("");
                    setRevokeTarget("others");
                  }}
                >
                  Sign out other sessions
                </button>
              </header>
              {sessions.isLoading && <p className="muted">Loading sessions…</p>}
              {sessions.isError && (
                <p className="alert error" role="alert">
                  Sessions could not be loaded.{" "}
                  <button
                    className="button ghost"
                    onClick={() => void sessions.refetch()}
                  >
                    Retry
                  </button>
                </p>
              )}
              {sessions.isSuccess && sessions.data.length === 0 && (
                <p className="muted">No session history yet.</p>
              )}
              {sessions.data && sessions.data.length > 0 && (
                <div
                  className="profile-session-table"
                  role="region"
                  aria-label="Session history"
                  tabIndex={0}
                >
                  <table className="responsive-data-table">
                    <thead>
                      <tr>
                        <th scope="col">Session</th>
                        <th scope="col">First seen</th>
                        <th scope="col">Last seen</th>
                        <th scope="col">Status</th>
                        <th scope="col">Remarks</th>
                        <th scope="col">Action</th>
                      </tr>
                    </thead>
                    <tbody>
                      {sessions.data.map((session) => (
                        <tr key={session.id}>
                          <td data-label="Session">
                            <div>
                              <strong>
                                {session.ip_address ?? "IP unavailable"}
                              </strong>
                              <p className="muted">
                                {deviceName(session.user_agent)} ·{" "}
                                {session.authentication_method === "oidc"
                                  ? "SSO"
                                  : "Local"}
                              </p>
                            </div>
                          </td>
                          <td data-label="First seen">
                            <RelativeDateTime value={session.created_at} />
                          </td>
                          <td data-label="Last seen">
                            <RelativeDateTime value={session.last_seen_at} />
                          </td>
                          <td data-label="Status">
                            <span
                              className={`badge ${session.status === "active" ? "success" : "neutral"}`}
                            >
                              {session.current
                                ? "This session"
                                : SESSION_STATUS_LABELS[session.status]}
                            </span>
                          </td>
                          <td data-label="Remarks">
                            <p className="muted">
                              {session.ended_at ? "Ended " : "Expires "}
                              <RelativeDateTime
                                value={session.ended_at ?? session.expires_at}
                              />
                            </p>
                          </td>
                          <td data-label="Action">
                            {!session.current &&
                              session.status === "active" && (
                                <button
                                  className="button ghost"
                                  disabled={busy}
                                  onClick={() => {
                                    setSessionError("");
                                    setRevokeTarget(session);
                                  }}
                                >
                                  Sign out session
                                </button>
                              )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          )}
        </div>
      </div>
      {revokeTarget && (
        <ConfirmDialog
          title={
            revokeTarget === "others"
              ? "Sign out other sessions?"
              : "Sign out this device?"
          }
          message="The selected sessions will need to sign in again. Your current session will stay signed in."
          confirmLabel="Sign out"
          pending={revoke.isPending}
          error={sessionError}
          onClose={() => setRevokeTarget(null)}
          onConfirm={() => {
            if (!busy) revoke.mutate(revokeTarget);
          }}
        />
      )}
      {blocker.state === "blocked" && (
        <ConfirmDialog
          title="Discard unsaved preferences?"
          message="Your display preference changes have not been saved."
          confirmLabel="Discard changes"
          onConfirm={() => {
            setPreferences(profile.preferences);
            setPreferencesError("");
            blocker.proceed();
          }}
          onClose={blocker.reset}
        />
      )}
    </>
  );
}
