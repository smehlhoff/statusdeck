import { useEffect, useState } from "react";

export const SYSTEM_REFRESH_MS = 30_000;
const SNAPSHOT_TOLERANCE_MS = SYSTEM_REFRESH_MS * 3;

export type HealthState =
  | "Healthy"
  | "Degraded"
  | "Unavailable"
  | "Unknown"
  | "Not configured"
  | "Stale";

export interface DiagnosticQuery {
  data: unknown;
  dataUpdatedAt: number;
  isError: boolean;
  failureCount: number;
}

export function useDiagnosticClock(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 5_000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

export function snapshotState(
  query: DiagnosticQuery,
  now: number,
  toleranceMs = SNAPSHOT_TOLERANCE_MS,
): HealthState {
  if (query.data === undefined)
    return query.isError ? "Unavailable" : "Unknown";
  if (
    query.isError ||
    query.failureCount > 0 ||
    now - query.dataUpdatedAt > toleranceMs
  )
    return "Stale";
  return "Healthy";
}

export function healthClass(status: HealthState): string {
  switch (status) {
    case "Healthy":
      return "operational";
    case "Degraded":
    case "Stale":
      return "warning";
    case "Unavailable":
      return "major_outage";
    default:
      return "neutral";
  }
}
