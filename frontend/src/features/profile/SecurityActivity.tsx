import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { SecurityActivity as Activity } from "../../api/types";
import { RelativeDateTime } from "../../components/RelativeDateTime";

const ACTIONS: Record<string, string> = {
  "oidc.settings_updated": "Single sign-on settings updated",
  "oidc.login_succeeded": "Signed in with SSO",
  "oidc.login_denied": "SSO sign-in rejected",
  "oidc.linked": "SSO identity linked; other sessions signed out",
  "oidc.disconnected": "SSO identity disconnected",
  "oidc.provider_revoked":
    "Provider logout received; matching SSO sessions revoked",
  "session.login_succeeded": "Signed in",
  "session.login_failed": "Unsuccessful sign-in attempt",
  "session.logout": "Signed out",
  "profile.password_changed": "Password changed",
  "profile.email_changed": "Login email changed",
  "profile.session_revoked": "A session was signed out",
  "profile.other_sessions_revoked": "Other sessions were signed out",
};

export function SecurityActivity() {
  const activity = useQuery({
    queryKey: queryKeys.securityActivity,
    queryFn: ({ signal }) =>
      api<Activity[]>("/api/v1/profile/security-activity", { signal }),
    staleTime: 0,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false,
  });
  return (
    <section
      className="card profile-section"
      aria-labelledby="profile-security-activity"
    >
      <header className="profile-section-heading">
        <h2 id="profile-security-activity">Security activity</h2>
        <p className="profile-help">
          Your latest 100 sign-in and account security events.
        </p>
      </header>
      {activity.isLoading && (
        <p className="muted" role="status">
          Loading security activity…
        </p>
      )}
      {activity.isError && (
        <p className="alert error" role="alert">
          Security activity could not be loaded.{" "}
          <button
            className="button ghost"
            onClick={() => void activity.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      {activity.isSuccess && activity.data.length === 0 && (
        <p className="muted">No security activity yet.</p>
      )}
      <ol className="security-activity-list">
        {activity.data?.map((event) => (
          <li key={event.id}>
            <span
              className={`security-activity-dot${event.action === "session.login_failed" ? " security-activity-warning" : ""}`}
              aria-hidden="true"
            />
            <strong>{ACTIONS[event.action] ?? "Account security event"}</strong>
            <RelativeDateTime value={event.created_at} />
          </li>
        ))}
      </ol>
    </section>
  );
}
