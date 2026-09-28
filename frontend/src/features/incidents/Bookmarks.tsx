import { PersonalIncidentFilters } from "./PersonalIncidentFilters";
import { useInfiniteQuery } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router-dom";
import { api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { BookmarkPage } from "../../api/types";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { LoadingDots } from "../../components/LoadingDots";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { humanizeIdentifier } from "../../utils/display";
import { selectSearchParams } from "../../utils/searchParams";
import { BookmarkButton } from "./BookmarkButton";

const FILTER_NAMES = ["q", "provider_id"] as const;

export function Bookmarks() {
  const [urlSearchParams] = useSearchParams();
  const searchParams = selectSearchParams(urlSearchParams, FILTER_NAMES);
  const filters = searchParams.toString();
  const query = useInfiniteQuery({
    queryKey: [...queryKeys.bookmarks, filters],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) => {
      const params = new URLSearchParams(searchParams);
      if (pageParam) params.set("cursor", pageParam);
      return api<BookmarkPage>(`/api/v1/bookmarks?${params}`, { signal });
    },
    getNextPageParam: (page) => page.next_cursor,
  });
  const items = query.data?.pages.flatMap((page) => page.items) ?? [];
  const providers = query.data?.pages[0]?.providers ?? [];

  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Bookmarks</h1>
          <p className="muted">
            Saved incidents and maintenance, newest bookmarks first.
          </p>
        </div>
        <Link className="button ghost" to="/incidents">
          Browse incidents
        </Link>
      </div>
      <PersonalIncidentFilters
        heading="Find bookmarks"
        placeholder="Title or provider reference"
        providers={providers}
      />
      {query.isLoading && (
        <LoadingSkeleton label="Loading bookmarks" rows={4} />
      )}
      {query.isError && (
        <EmptyState
          title="Bookmarks unavailable"
          description="Bookmarks could not be loaded. Check the connection and try again."
          error
          inline
          action={
            <button
              className="button ghost"
              type="button"
              disabled={query.isFetching}
              onClick={() => {
                if (query.isFetchNextPageError) void query.fetchNextPage();
                else void query.refetch();
              }}
            >
              Retry
            </button>
          }
        />
      )}
      {query.isSuccess && items.length === 0 && (
        <EmptyState
          title={filters ? "No matching bookmarks" : "No bookmarks yet"}
          description={
            filters
              ? "Try a different search or provider, or clear the filters."
              : "Bookmark an incident or maintenance event to find it here later."
          }
          inline
        />
      )}
      <div className="timeline">
        {items.map((item) => (
          <article
            className={`card incident-row ${item.severity}`}
            key={item.id}
          >
            <div className="incident-row-main">
              <div className="badge-row">
                {item.providers.map((provider) => (
                  <span className="badge provider-badge" key={provider.id}>
                    {provider.name}
                  </span>
                ))}
                <span className={`badge ${item.severity}`}>
                  {item.severity}
                </span>
                <span className="badge neutral">
                  {humanizeIdentifier(item.lifecycle)}
                </span>
                <span className="badge neutral">{item.kind}</span>
              </div>
              <h2>
                <Link
                  to={`/incidents/${item.id}`}
                  state={{ bookmarkSearch: filters }}
                >
                  {item.title}
                </Link>
              </h2>
              <p className="muted incident-row-footer">
                Bookmarked <RelativeDateTime value={item.bookmarked_at} />
              </p>
            </div>
            <BookmarkButton incidentId={item.id} bookmarked />
          </article>
        ))}
      </div>
      {query.hasNextPage && (
        <button
          className="button ghost"
          disabled={query.isFetching}
          onClick={() => void query.fetchNextPage()}
        >
          {query.isFetchingNextPage ? (
            <LoadingDots label="Loading more bookmarks" />
          ) : (
            "Load more bookmarks"
          )}
        </button>
      )}
    </>
  );
}
