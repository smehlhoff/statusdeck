import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import { queryKeys, LIVE_DATA_REFRESH_INTERVAL_MS } from "../../api/queries";
import type { ProviderReliability } from "../../api/types";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { resolvedTimeZone } from "../../utils/display";
import { useProfile } from "../profile/profileContext";
import { ReliabilityCharts } from "./ReliabilityCharts";

export function ReliabilityHistory({
  providerId,
  components,
}: {
  providerId: string;
  components: Array<{ id: string; name: string }>;
}) {
  const { preferences } = useProfile();
  const timeZone = resolvedTimeZone(preferences);
  const [period, setPeriod] = useState(365);
  const [component, setComponent] = useState("all");
  const history = useQuery({
    queryKey: queryKeys.providerReliability(
      providerId,
      period,
      component,
      timeZone,
    ),
    queryFn: ({ signal }) => {
      const params = new URLSearchParams({ days: String(period) });
      if (component !== "all") params.set("component_id", component);
      params.set("time_zone", timeZone);
      return api<ProviderReliability>(
        `/api/v1/catalog/providers/${providerId}/reliability?${params}`,
        { signal },
      );
    },
    refetchInterval: LIVE_DATA_REFRESH_INTERVAL_MS,
    refetchIntervalInBackground: false,
  });
  if (!history.data)
    return (
      <section className="card provider-detail-section">
        <h2>Reliability history</h2>
        {history.isError ? (
          <p className="alert error" role="alert">
            History could not be loaded.{" "}
            <button
              className="button ghost"
              onClick={() => void history.refetch()}
            >
              Retry
            </button>
          </p>
        ) : (
          <LoadingSkeleton label="Loading reliability history" rows={3} />
        )}
      </section>
    );
  return (
    <>
      {history.isError && (
        <p className="alert error" role="alert">
          History refresh failed. Showing the last successful result.{" "}
          <button
            className="button ghost"
            onClick={() => void history.refetch()}
          >
            Retry
          </button>
        </p>
      )}
      <ReliabilityCharts
        data={history.data}
        components={components}
        period={period}
        onPeriod={setPeriod}
        component={component}
        onComponent={setComponent}
        providerId={providerId}
      />
    </>
  );
}
