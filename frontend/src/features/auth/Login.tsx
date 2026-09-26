import type { FormEvent } from "react";
import { useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { ApiFailure, api, ensureCsrf } from "../../api/client";
import type { User } from "../../api/types";

export function Login({ onLogin }: { onLogin: (user: User) => Promise<void> }) {
  const navigate = useNavigate();
  const location = useLocation();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError("");
    setBusy(true);
    try {
      await ensureCsrf();
      const user = await api<User>("/api/v1/session", {
        method: "POST",
        body: JSON.stringify({ email, password }),
      });
      await onLogin(user);
      if (
        location.pathname === "/login" ||
        (location.pathname === "/" && !location.search && !location.hash)
      ) {
        navigate(user.preferences.landing_page, { replace: true });
      }
    } catch (error) {
      setError(
        error instanceof ApiFailure
          ? error.message
          : "StatusDeck could not be reached. Try again after the service recovers.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="login-page">
      <form className="card login-card" onSubmit={submit}>
        <p className="eyebrow">Provider-reported health</p>
        <h1>Sign in to StatusDeck</h1>
        {error && (
          <div className="alert error" role="alert">
            {error}
          </div>
        )}
        <label>
          Email
          <input
            type="email"
            autoComplete="username"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            required
          />
        </label>
        <label>
          Password
          <input
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            required
          />
        </label>
        <button className="button primary wide" disabled={busy}>
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </main>
  );
}
