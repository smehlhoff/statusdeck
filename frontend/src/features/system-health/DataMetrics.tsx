import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { RecordCount } from "../../api/types";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { humanizeIdentifier } from "../../utils/display";
import { snapshotState, SYSTEM_REFRESH_MS } from "./diagnosticState";

const RECORD_TYPES: Record<string, { label: string; description: string }> = {
  incidents: {
    label: "Incidents",
    description: "Distinct provider incidents, including imported history.",
  },
  maintenance: {
    label: "Maintenance",
    description: "Scheduled maintenance events.",
  },
  incident_updates: {
    label: "Incident updates",
    description:
      "Updates for incidents and maintenance, including synthesized updates.",
  },
  incident_comments: {
    label: "Internal comments",
    description: "Comments added by StatusDeck users.",
  },
  provider_status_changes: {
    label: "Provider status changes",
    description: "Recorded changes to provider provider status.",
  },
  component_status_changes: {
    label: "Component status changes",
    description: "Recorded changes to individual component status.",
  },
  poll_runs: {
    label: "Provider checks",
    description:
      "Polling attempts, including failures and unchanged responses.",
  },
  poll_payloads: {
    label: "Stored poll payloads",
    description:
      "Captured responses and failure evidence within the retention period.",
  },
  notification_events: {
    label: "Notification events",
    description:
      "Generated events, including channel tests and quiet-hours summaries.",
  },
  notification_deliveries: {
    label: "Notification deliveries",
    description: "Delivery records across all channels and statuses.",
  },
  notification_attempts: {
    label: "Delivery attempts",
    description: "Individual send attempts, including retries.",
  },
};

export function DataMetrics({ now }: { now: number }) {
  const metrics = useQuery({
    queryKey: queryKeys.dataMetrics,
    queryFn: ({ signal }) =>
      api<RecordCount[]>("/api/v1/system/data-metrics", { signal }),
    staleTime: SYSTEM_REFRESH_MS,
    refetchInterval: SYSTEM_REFRESH_MS,
    refetchIntervalInBackground: false,
  });

  const state = snapshotState(metrics, now, SYSTEM_REFRESH_MS * 3);

  return (
    <section
      className="card diagnostics-section"
      aria-labelledby="data-metrics-heading"
    >
      <div className="section-heading">
        <h2 id="data-metrics-heading">Data metrics</h2>
      </div>
      {metrics.isLoading && (
        <LoadingSkeleton label="Loading data metrics" inline />
      )}
      {(state === "Stale" || state === "Unavailable") && (
        <p className="alert error" role="alert">
          {metrics.data
            ? "Data metrics could not be refreshed. Showing the last successful counts."
            : "Data metrics could not be loaded. Retrying automatically."}
        </p>
      )}
      {metrics.data && (
        <div
          className="table-wrap"
          role="region"
          aria-label="Stored records by type"
          tabIndex={0}
        >
          <table className="data-table responsive-data-table">
            <thead>
              <tr>
                <th scope="col">Record type</th>
                <th scope="col">Stored records</th>
                <th scope="col">What is counted</th>
              </tr>
            </thead>
            <tbody>
              {metrics.data.map((record) => (
                <tr key={record.record_type}>
                  <td data-label="Record type">
                    {RECORD_TYPES[record.record_type]?.label ??
                      humanizeIdentifier(record.record_type)}
                  </td>
                  <td data-label="Stored records">
                    {record.count.toLocaleString()}
                  </td>
                  <td data-label="What is counted">
                    {RECORD_TYPES[record.record_type]?.description ??
                      "Retained records."}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
