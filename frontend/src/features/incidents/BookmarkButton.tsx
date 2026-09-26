import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api, ensureCsrf } from "../../api/client";
import { queryKeys } from "../../api/queries";
import { useToast } from "../../components/toastContext";

export function BookmarkButton({
  incidentId,
  bookmarked,
}: {
  incidentId: string;
  bookmarked: boolean;
}) {
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const mutation = useMutation({
    mutationFn: async () => {
      await ensureCsrf();
      await api(`/api/v1/incidents/${incidentId}/bookmark`, {
        method: bookmarked ? "DELETE" : "PUT",
      });
    },
    onSuccess: async () => {
      notify(bookmarked ? "Bookmark removed." : "Incident bookmarked.");
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.bookmarks }),
        queryClient.invalidateQueries({ queryKey: queryKeys.incidents }),
        queryClient.invalidateQueries({
          queryKey: queryKeys.incidentDetail(incidentId),
        }),
        queryClient.invalidateQueries({ queryKey: queryKeys.catalog }),
      ]);
    },
    onError: () => notify("Could not update the bookmark. Try again.", "error"),
  });

  const label = bookmarked ? "Bookmarked" : "Bookmark";
  return (
    <button
      className="button ghost"
      type="button"
      aria-label={bookmarked ? "Remove bookmark" : "Bookmark incident"}
      aria-pressed={bookmarked}
      disabled={mutation.isPending}
      onClick={() => mutation.mutate()}
    >
      {mutation.isPending ? "Saving…" : label}
    </button>
  );
}
