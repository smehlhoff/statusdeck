import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { queryKeys } from "../api/queries";
import type { DisplayPreferences, User } from "../api/types";
import { useProfile } from "../features/profile/profileContext";
import { useToast } from "../components/toastContext";

const REFRESH_OPTIONS = [0, 5_000, 15_000, 60_000] as const;

export function useRefreshInterval() {
  const { preferences } = useProfile();
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const save = useMutation({
    mutationFn: (
      refresh_interval_ms: DisplayPreferences["refresh_interval_ms"],
    ) =>
      api<User>("/api/v1/profile", {
        method: "PATCH",
        body: JSON.stringify({ preferences: { refresh_interval_ms } }),
      }),
    onSuccess: async (profile) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.session });
      queryClient.setQueryData(queryKeys.session, profile);
    },
    onError: () =>
      notify("The refresh preference could not be saved. Try again.", "error"),
  });

  function cycleRefreshInterval() {
    if (save.isPending) return;
    const currentIndex = REFRESH_OPTIONS.indexOf(
      preferences.refresh_interval_ms,
    );
    save.mutate(REFRESH_OPTIONS[(currentIndex + 1) % REFRESH_OPTIONS.length]);
  }

  return {
    refreshInterval: preferences.refresh_interval_ms,
    cycleRefreshInterval,
  };
}
