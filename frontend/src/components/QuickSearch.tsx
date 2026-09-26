import { useQuery } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Link, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { queryKeys } from "../api/queries";
import type { CatalogProvider, IncidentPage } from "../api/types";
import { humanizeIdentifier } from "../utils/display";

const RESULT_LIMIT = 6;
const SEARCH_DELAY_MS = 250;
const PAGES = [
  {
    title: "Bookmarks",
    detail: "Saved incidents and maintenance",
    href: "/bookmarks",
  },
  {
    title: "My Comments",
    detail: "Your incident comments",
    href: "/my-comments",
  },
  { title: "Overview", detail: "Dashboard and wall mode", href: "/" },
  { title: "Providers", detail: "Catalog and monitoring", href: "/catalog" },
  { title: "Incidents", detail: "Incident history", href: "/incidents" },
  {
    title: "Analytics",
    detail: "Reliability and restoration",
    href: "/analytics",
  },
  {
    title: "Notifications",
    detail: "Channels and alert rules",
    href: "/notifications",
  },
  { title: "System", detail: "Polling and system health", href: "/system" },
  {
    title: "Profile",
    detail: "Account, preferences, and sessions",
    href: "/profile",
  },
];

interface SearchResult {
  title: string;
  detail: string;
  href: string;
  group: "Pages" | "Providers" | "Incidents";
}

export function QuickSearch() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    function showSearch() {
      if (!document.querySelector("dialog[open]")) setOpen(true);
    }

    function shortcut(event: KeyboardEvent) {
      if (
        (event.ctrlKey || event.metaKey) &&
        event.key.toLowerCase() === "k" &&
        !event.altKey
      ) {
        event.preventDefault();
        if (!event.repeat) showSearch();
      }
    }

    window.addEventListener("keydown", shortcut);
    window.addEventListener("statusdeck:open-search", showSearch);
    return () => {
      window.removeEventListener("keydown", shortcut);
      window.removeEventListener("statusdeck:open-search", showSearch);
    };
  }, []);

  return (
    <>
      <button
        type="button"
        className="quick-search-trigger"
        onClick={() => setOpen(true)}
        aria-keyshortcuts="Control+k Meta+k"
      >
        <span className="quick-search-label">
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.75"
            strokeLinecap="round"
            aria-hidden="true"
          >
            <circle cx="10.5" cy="10.5" r="6.5" />
            <path d="m16 16 4.5 4.5" />
          </svg>
          Search
        </span>
        <kbd>Ctrl / ⌘ K</kbd>
      </button>
      {open &&
        createPortal(
          <QuickSearchDialog onClose={() => setOpen(false)} />,
          document.body,
        )}
    </>
  );
}

function QuickSearchDialog({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const id = useId();
  const [text, setText] = useState("");
  const [debouncedTerm, setDebouncedTerm] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const term = text.trim();

  useEffect(() => {
    const previousFocus = document.activeElement;
    const dialog = dialogRef.current;
    dialog?.showModal();
    inputRef.current?.focus();
    return () => {
      dialog?.close();
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected)
        previousFocus.focus();
    };
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(
      () => setDebouncedTerm(term),
      SEARCH_DELAY_MS,
    );
    return () => window.clearTimeout(timer);
  }, [term]);

  const providers = useQuery({
    queryKey: queryKeys.catalog,
    queryFn: ({ signal }) =>
      api<CatalogProvider[]>("/api/v1/catalog/providers", { signal }),
  });
  const incidents = useQuery({
    queryKey: queryKeys.quickSearchIncidents(debouncedTerm),
    queryFn: ({ signal }) => {
      const params = new URLSearchParams({
        q: debouncedTerm,
        scope: "all",
        limit: String(RESULT_LIMIT),
      });
      return api<IncidentPage>(`/api/v1/incidents?${params}`, { signal });
    },
    enabled: term.length >= 2 && term === debouncedTerm,
  });

  const needle = term.toLocaleLowerCase();
  const results: SearchResult[] = PAGES.filter((page) =>
    `${page.title} ${page.detail}`.toLocaleLowerCase().includes(needle),
  ).map((page) => ({ ...page, group: "Pages" }));
  if (term.length >= 2) {
    results.push(
      ...(providers.data ?? [])
        .filter((provider) =>
          `${provider.name} ${provider.slug} ${provider.tags.join(" ")}`
            .toLocaleLowerCase()
            .includes(needle),
        )
        .slice(0, RESULT_LIMIT)
        .map((provider): SearchResult => ({
          title: provider.name,
          detail: provider.tags.join(" · "),
          href: `/catalog/${provider.id}`,
          group: "Providers",
        })),
    );
    if (term === debouncedTerm) {
      results.push(
        ...(incidents.data?.items ?? []).map((incident): SearchResult => ({
          title: incident.title,
          detail: `${incident.providers.map((provider) => provider.name).join(", ")} · ${humanizeIdentifier(incident.original_phase)}`,
          href: `/incidents/${incident.id}`,
          group: "Incidents",
        })),
      );
    }
  }
  const selectedIndex = Math.min(activeIndex, Math.max(results.length - 1, 0));
  const selectedResult = results[selectedIndex];
  const searching =
    term.length >= 2 &&
    (term !== debouncedTerm || incidents.isFetching || providers.isFetching);

  useEffect(() => {
    document
      .getElementById(`${id}-result-${selectedIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [id, selectedIndex, selectedResult?.href]);

  function choose(result: SearchResult) {
    onClose();
    navigate(result.href);
  }

  return (
    <dialog
      ref={dialogRef}
      className="quick-search-dialog"
      aria-labelledby={`${id}-title`}
      onCancel={onClose}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="section-heading">
        <h2 id={`${id}-title`}>Quick search</h2>
        <button
          type="button"
          className="button ghost compact"
          onClick={onClose}
        >
          Close
        </button>
      </div>
      <input
        ref={inputRef}
        type="search"
        role="combobox"
        aria-label="Search providers, incidents, and pages"
        aria-controls={`${id}-results`}
        aria-expanded="true"
        aria-autocomplete="list"
        aria-activedescendant={
          selectedResult ? `${id}-result-${selectedIndex}` : undefined
        }
        autoComplete="off"
        maxLength={200}
        placeholder="Search providers, incidents, or pages"
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          setActiveIndex(0);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (results.length)
              setActiveIndex(
                (selectedIndex +
                  (event.key === "ArrowDown" ? 1 : -1) +
                  results.length) %
                  results.length,
              );
          } else if (event.key === "Enter" && selectedResult) {
            event.preventDefault();
            choose(selectedResult);
          }
        }}
      />
      {term.length >= 2 && (
        <p className="quick-search-status muted" role="status">
          {searching ? "Searching…" : `${results.length} results`}
        </p>
      )}
      <div
        id={`${id}-results`}
        className="quick-search-results"
        role="listbox"
        aria-label="Search results"
        aria-busy={searching}
      >
        {(["Pages", "Providers", "Incidents"] as const).map((group) => {
          const matches = results
            .map((result, index) => ({ result, index }))
            .filter(({ result }) => result.group === group);
          if (!matches.length) return null;
          return (
            <div role="group" aria-label={group} key={group}>
              <h3 aria-hidden="true">{group}</h3>
              {matches.map(({ result, index }) => (
                <Link
                  key={result.href}
                  id={`${id}-result-${index}`}
                  to={result.href}
                  role="option"
                  aria-selected={index === selectedIndex}
                  tabIndex={-1}
                  className="quick-search-result"
                  onMouseMove={() => setActiveIndex(index)}
                  onClick={onClose}
                >
                  <strong>{result.title}</strong>
                  <span>{result.detail}</span>
                </Link>
              ))}
            </div>
          );
        })}
      </div>
      {term.length >= 2 &&
        !searching &&
        !results.length &&
        !providers.isError &&
        !incidents.isError && (
          <p className="muted">
            No matches. Try a provider name, incident title, or incident ID.
          </p>
        )}
      {providers.isError && (
        <p role="alert">
          Providers could not be loaded.{" "}
          <button
            className="button ghost compact"
            onClick={() => void providers.refetch()}
          >
            Retry providers
          </button>
        </p>
      )}
      {term.length >= 2 && term === debouncedTerm && incidents.isError && (
        <p role="alert">
          Incidents could not be searched.{" "}
          <button
            className="button ghost compact"
            onClick={() => void incidents.refetch()}
          >
            Retry incidents
          </button>
        </p>
      )}
      <div className="quick-search-footer">
        <span className="muted">
          ↑ ↓ to select · Enter to open · Esc to close
        </span>
        {term.length >= 2 && (
          <Link
            to={`/incidents?${new URLSearchParams({ q: term, scope: "all" })}`}
            onClick={onClose}
          >
            View all incident matches
          </Link>
        )}
      </div>
    </dialog>
  );
}
