import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useId, useRef, useState } from "react";
import { useBeforeUnload, useBlocker } from "react-router-dom";
import { ApiFailure, api } from "../../api/client";
import { LIVE_DATA_REFRESH_INTERVAL_MS, queryKeys } from "../../api/queries";
import type { IncidentComment, IncidentCommentPage } from "../../api/types";
import { UserAvatar } from "../../components/UserAvatar";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { EmptyState } from "../../components/EmptyState";
import { LoadingSkeleton } from "../../components/LoadingSkeleton";
import { LoadingDots } from "../../components/LoadingDots";
import { RelativeDateTime } from "../../components/RelativeDateTime";
import { useToast } from "../../components/toastContext";
import { CommentMarkdown } from "./CommentMarkdown";
import { CommentEditor } from "./CommentEditor";

const MAX_COMMENT_CHARACTERS = 5_000;

function validComment(body: string): boolean {
  const trimmed = body.trim();
  return (
    trimmed.length > 0 &&
    Array.from(trimmed).length <= MAX_COMMENT_CHARACTERS &&
    !trimmed.includes("\0")
  );
}

function errorMessage(error: unknown): string {
  return error instanceof ApiFailure
    ? error.message
    : "The comment could not be saved. Check the connection and try again.";
}

export function IncidentComments({ incidentId }: { incidentId: string }) {
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const headingId = useId();
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useState("");
  const [draftMode, setDraftMode] = useState<"write" | "preview">("write");
  const [editMode, setEditMode] = useState<"write" | "preview">("write");
  const [edit, setEdit] = useState<{
    id: string;
    body: string;
    originalBody: string;
  } | null>(null);
  const hasUnsavedChanges =
    draft !== "" || (edit !== null && edit.body !== edit.originalBody);
  const blocker = useBlocker(hasUnsavedChanges);

  useBeforeUnload(
    useCallback(
      (event) => {
        if (!hasUnsavedChanges) return;
        event.preventDefault();
        event.returnValue = "";
      },
      [hasUnsavedChanges],
    ),
  );
  const [toDelete, setToDelete] = useState<IncidentComment | null>(null);
  const [formError, setFormError] = useState("");
  const [editError, setEditError] = useState("");
  const [deleteError, setDeleteError] = useState("");
  const basePath = `/api/v1/incidents/${incidentId}/comments`;
  const queryKey = queryKeys.incidentComments(incidentId);
  const query = useInfiniteQuery({
    queryKey,
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) =>
      api<IncidentCommentPage>(
        `${basePath}${pageParam ? `?cursor=${encodeURIComponent(pageParam)}` : ""}`,
        { signal },
      ),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    refetchInterval: (query) =>
      (query.state.data?.pages.length ?? 0) <= 1
        ? LIVE_DATA_REFRESH_INTERVAL_MS
        : false,
    refetchIntervalInBackground: false,
  });

  async function refreshComments() {
    await queryClient.cancelQueries({ queryKey });
    await Promise.all([
      queryClient.invalidateQueries({ queryKey }),
      queryClient.invalidateQueries({ queryKey: queryKeys.myComments }),
    ]);
  }

  const create = useMutation({
    mutationFn: (body: string) =>
      api<IncidentComment>(basePath, {
        method: "POST",
        body: JSON.stringify({ body }),
      }),
    onSuccess: async () => {
      setDraft("");
      setDraftMode("write");
      setFormError("");
      notify("Comment added.");
      await refreshComments();
    },
    onError: (error) => setFormError(errorMessage(error)),
  });
  const update = useMutation({
    mutationFn: (comment: { id: string; body: string }) =>
      api<IncidentComment>(`${basePath}/${comment.id}`, {
        method: "PATCH",
        body: JSON.stringify({ body: comment.body }),
      }),
    onSuccess: async () => {
      setEdit(null);
      setEditError("");
      notify("Comment updated.");
      composerRef.current?.focus();
      await refreshComments();
    },
    onError: (error) => setEditError(errorMessage(error)),
  });
  const remove = useMutation({
    mutationFn: (id: string) =>
      api<void>(`${basePath}/${id}`, { method: "DELETE" }),
    onSuccess: async () => {
      setToDelete(null);
      setDeleteError("");
      notify("Comment deleted.");
      composerRef.current?.focus();
      await refreshComments();
    },
    onError: (error) =>
      setDeleteError(
        error instanceof ApiFailure
          ? error.message
          : "The comment could not be deleted. Check the connection and try again.",
      ),
  });
  const pending = create.isPending || update.isPending || remove.isPending;
  const comments = query.data?.pages.flatMap((page) => page.items) ?? [];
  const commentCount = query.data?.pages[0].total_count;

  return (
    <section className="card incident-comments" aria-labelledby={headingId}>
      <div className="section-heading">
        <div>
          <h2 id={headingId}>Internal comments</h2>
          <p className="comments-intro">
            Capture impact, workarounds, and follow-up notes for this incident.
          </p>
        </div>
        {commentCount !== undefined && (
          <span className="badge neutral comment-count">
            {commentCount} {commentCount === 1 ? "Comment" : "Comments"}
          </span>
        )}
      </div>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (!pending && validComment(draft)) {
            setFormError("");
            create.mutate(draft.trim());
          }
        }}
      >
        <CommentEditor
          textareaRef={composerRef}
          value={draft}
          onChange={setDraft}
          mode={draftMode}
          onModeChange={setDraftMode}
          disabled={create.isPending}
          label="Internal comment"
        />
        <div className="comment-composer-footer">
          <button
            className="button primary"
            disabled={pending || !validComment(draft)}
          >
            {create.isPending ? "Adding…" : "Add comment"}
          </button>
        </div>
        {Array.from(draft.trim()).length > MAX_COMMENT_CHARACTERS && (
          <p className="alert error" role="alert">
            Keep your comment within 5,000 characters.
          </p>
        )}
        {formError && (
          <p className="alert error" role="alert">
            {formError}
          </p>
        )}
      </form>
      {query.isLoading && <LoadingSkeleton label="Loading comments" rows={2} />}
      {query.isError && (
        <div className="alert error" role="alert">
          {query.data
            ? "Comments could not be refreshed. Showing the last loaded comments."
            : "Comments could not be loaded."}{" "}
          <button
            className="button ghost"
            disabled={query.isFetching}
            onClick={() =>
              void (query.isFetchNextPageError
                ? query.fetchNextPage()
                : query.refetch())
            }
          >
            Retry
          </button>
        </div>
      )}
      {query.isSuccess && comments.length === 0 && (
        <EmptyState
          title="No internal comments yet"
          description="Add the first comment about your response to this incident."
          inline
        />
      )}
      <div className="comment-list">
        {comments.map((comment) => (
          <article className="incident-comment" key={comment.id}>
            <UserAvatar name={comment.author_display_name} />
            <div className="comment-content">
              <div className="comment-meta">
                <strong title={comment.author_email}>
                  {comment.author_display_name}
                </strong>
                <span className="comment-admin-tag">Admin</span>
                <RelativeDateTime value={comment.created_at} />
                {comment.edited_at && (
                  <span className="comment-edited">
                    ·{" "}
                    <RelativeDateTime
                      value={comment.edited_at}
                      prefix="edited "
                    />
                  </span>
                )}
              </div>
              {edit?.id === comment.id ? (
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    if (!pending && validComment(edit.body)) {
                      setEditError("");
                      update.mutate({ id: edit.id, body: edit.body.trim() });
                    }
                  }}
                >
                  <CommentEditor
                    value={edit.body}
                    onChange={(body) => setEdit({ ...edit, body })}
                    mode={editMode}
                    onModeChange={setEditMode}
                    disabled={update.isPending}
                    label="Edit comment"
                    autoFocus
                  />
                  <p className="muted comment-edit-help">
                    Up to 5,000 characters.
                  </p>
                  {editError && (
                    <p className="alert error" role="alert">
                      {editError}
                    </p>
                  )}
                  <div className="comment-actions">
                    <button
                      className="button ghost"
                      disabled={pending || !validComment(edit.body)}
                    >
                      {update.isPending ? "Saving…" : "Save changes"}
                    </button>
                    <button
                      className="button ghost"
                      type="button"
                      disabled={pending}
                      onClick={() => {
                        setEdit(null);
                        setEditError("");
                        composerRef.current?.focus();
                      }}
                    >
                      Cancel
                    </button>
                  </div>
                </form>
              ) : (
                <>
                  <CommentMarkdown body={comment.body} />
                  <div className="comment-actions">
                    <button
                      type="button"
                      disabled={pending || edit !== null}
                      onClick={() => {
                        setEditError("");
                        setEditMode("write");
                        setEdit({
                          id: comment.id,
                          body: comment.body,
                          originalBody: comment.body,
                        });
                      }}
                    >
                      Edit
                    </button>
                    <button
                      className="comment-delete"
                      type="button"
                      disabled={pending || edit !== null}
                      onClick={() => {
                        setDeleteError("");
                        setToDelete(comment);
                      }}
                    >
                      Delete
                    </button>
                  </div>
                </>
              )}
            </div>
          </article>
        ))}
      </div>
      {query.hasNextPage && (
        <button
          className="button ghost"
          disabled={query.isFetching || pending}
          onClick={() => void query.fetchNextPage()}
        >
          {query.isFetchingNextPage ? (
            <LoadingDots label="Loading older comments" />
          ) : (
            "Load older comments"
          )}
        </button>
      )}
      {blocker.state === "blocked" && (
        <ConfirmDialog
          title="Discard unsaved changes?"
          message="Your comment draft or edits have not been saved."
          confirmLabel="Discard changes"
          onConfirm={blocker.proceed}
          onClose={blocker.reset}
        />
      )}
      {toDelete && (
        <ConfirmDialog
          title="Delete comment?"
          message="This comment will be permanently removed from the incident."
          confirmLabel="Delete comment"
          pending={remove.isPending}
          error={deleteError}
          onConfirm={() => {
            if (!pending) {
              setDeleteError("");
              remove.mutate(toDelete.id);
            }
          }}
          onClose={() => setToDelete(null)}
        />
      )}
    </section>
  );
}
