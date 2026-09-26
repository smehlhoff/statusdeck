import type { QueryClient } from "@tanstack/react-query";

export const LIVE_DATA_REFRESH_INTERVAL_MS = 60_000;

export const queryKeys = {
  session: ["session"] as const,
  myComments: ["my-comments"] as const,
  bookmarks: ["bookmarks"] as const,
  profileSessions: ["profile-sessions"] as const,
  securityActivity: ["security-activity"] as const,
  catalog: ["catalog"] as const,
  catalogDetail: (id: string | undefined) => ["catalog", id] as const,
  providerReliability: (
    id: string,
    days: number,
    component: string,
    timeZone: string,
  ) => ["catalog", id, "reliability", days, component, timeZone] as const,
  catalogComponents: (providerIds: string[]) =>
    ["catalog-components", providerIds] as const,
  dashboard: ["dashboard"] as const,
  analytics: (filters: string) => ["analytics", filters] as const,
  incidents: ["incidents"] as const,
  incidentList: (filters: string) => ["incidents", filters] as const,
  quickSearchIncidents: (term: string) =>
    ["incidents", "quick-search", term] as const,
  incidentDetail: (id: string | undefined) => ["incident", id] as const,
  incidentComments: (id: string) => ["incident-comments", id] as const,
  monitors: ["monitors"] as const,
  channels: ["channels"] as const,
  rules: ["rules"] as const,
  notificationSummary: (id: string | undefined) =>
    ["notification-summary", id] as const,
  deliveries: ["deliveries"] as const,
  recentDeliveries: ["deliveries", "system-recent"] as const,
  sources: ["sources"] as const,
  systemSummary: ["system-summary"] as const,
  dataMetrics: ["system-data-metrics"] as const,
};

export async function invalidateMonitoringState(
  queryClient: QueryClient,
): Promise<void> {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.monitors }),
    queryClient.invalidateQueries({ queryKey: queryKeys.dashboard }),
    queryClient.invalidateQueries({ queryKey: queryKeys.incidents }),
    queryClient.invalidateQueries({ queryKey: queryKeys.catalog }),
    queryClient.invalidateQueries({ queryKey: ["analytics"] }),
    queryClient.invalidateQueries({ queryKey: ["catalog-components"] }),
    queryClient.invalidateQueries({ queryKey: queryKeys.sources }),
    queryClient.invalidateQueries({ queryKey: queryKeys.systemSummary }),
  ]);
}
