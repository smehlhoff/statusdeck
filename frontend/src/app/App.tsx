import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from "@tanstack/react-query";
import {
  Link,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
} from "react-router-dom";
import { ApiFailure, api, ensureCsrf } from "../api/client";
import { queryKeys } from "../api/queries";
import type { User } from "../api/types";
import { ToastProvider } from "../components/ToastProvider";
import { LoadingSkeleton } from "../components/LoadingSkeleton";
import { QuickSearch } from "../components/QuickSearch";
import { Login } from "../features/auth/Login";
import { ProfileContext } from "../features/profile/profileContext";
import { UserAvatar } from "../components/UserAvatar";
import { setThemePreference } from "../features/profile/theme";
import { NavigationIcon } from "./NavigationIcon";
import "../styles/global.css";

const MyComments = lazy(() =>
  import("../features/incidents/MyComments").then((module) => ({
    default: module.MyComments,
  })),
);

const Bookmarks = lazy(() =>
  import("../features/incidents/Bookmarks").then((module) => ({
    default: module.Bookmarks,
  })),
);

const Profile = lazy(() =>
  import("../features/profile/Profile").then((module) => ({
    default: module.Profile,
  })),
);

const Dashboard = lazy(() =>
  import("../features/dashboard/Dashboard").then((module) => ({
    default: module.Dashboard,
  })),
);
const Catalog = lazy(() =>
  import("../features/catalog/Catalog").then((module) => ({
    default: module.Catalog,
  })),
);
const ProviderDetail = lazy(() =>
  import("../features/catalog/ProviderDetail").then((module) => ({
    default: module.ProviderDetail,
  })),
);
const Incidents = lazy(() =>
  import("../features/incidents/Incidents").then((module) => ({
    default: module.Incidents,
  })),
);
const IncidentDetail = lazy(() =>
  import("../features/incidents/IncidentDetail").then((module) => ({
    default: module.IncidentDetail,
  })),
);
const Analytics = lazy(() =>
  import("../features/analytics/Analytics").then((module) => ({
    default: module.Analytics,
  })),
);
const Notifications = lazy(() =>
  import("../features/notifications/Notifications").then((module) => ({
    default: module.Notifications,
  })),
);
const SystemHealth = lazy(() =>
  import("../features/system-health/SystemHealth").then((module) => ({
    default: module.SystemHealth,
  })),
);
const NotificationSummary = lazy(() =>
  import("../features/notifications/NotificationSummary").then((module) => ({
    default: module.NotificationSummary,
  })),
);

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      retry: (failureCount, error) =>
        failureCount < 1 &&
        !(
          error instanceof ApiFailure &&
          error.status >= 400 &&
          error.status < 500
        ),
    },
  },
});

function pageTitle(pathname: string): string {
  if (pathname.startsWith("/notifications/summaries/"))
    return "Quiet-Hours Summary";
  if (pathname.startsWith("/catalog/")) return "Provider details";
  if (pathname === "/catalog") return "Providers";
  if (pathname.startsWith("/incidents/")) return "Incident details";
  if (pathname === "/incidents") return "Incidents";
  if (pathname.startsWith("/analytics")) return "Analytics";
  if (pathname.startsWith("/notifications")) return "Notifications";
  if (pathname.startsWith("/system")) return "System";
  if (pathname === "/my-comments") return "My Comments";
  if (pathname === "/bookmarks") return "Bookmarks";
  if (pathname === "/profile") return "Profile";
  return "Overview";
}

function Shell({
  user,
  setUser,
}: {
  user: User;
  setUser: (user: User | null) => void;
}) {
  const navigate = useNavigate();
  const location = useLocation();
  const [logoutError, setLogoutError] = useState("");
  const [loggingOut, setLoggingOut] = useState(false);
  const [accountOpen, setAccountOpen] = useState(false);
  const accountRef = useRef<HTMLDivElement>(null);
  const accountButtonRef = useRef<HTMLButtonElement>(null);
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (!accountOpen) return;

    function closeOutside(event: PointerEvent) {
      if (
        event.target instanceof Node &&
        !accountRef.current?.contains(event.target)
      ) {
        setAccountOpen(false);
      }
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setAccountOpen(false);
        accountButtonRef.current?.focus();
      }
    }

    document.addEventListener("pointerdown", closeOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [accountOpen]);

  useLayoutEffect(() => {
    document.title = `StatusDeck · ${pageTitle(location.pathname)}`;
  }, [location.pathname]);

  useEffect(() => {
    window.scrollTo({ top: 0, left: 0 });
    mainRef.current?.focus({ preventScroll: true });
  }, [location.pathname]);

  async function logout() {
    setLogoutError("");
    setLoggingOut(true);
    try {
      await ensureCsrf();
      await api("/api/v1/session", { method: "DELETE" });
      queryClient.clear();
      setUser(null);
      navigate("/login");
    } catch {
      setLogoutError(
        "StatusDeck could not log you out. Check the connection and try again.",
      );
    } finally {
      setLoggingOut(false);
    }
  }

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to content
      </a>
      <header className="topbar">
        <div className="topbar-inner">
          <Link className="brand" to="/">
            <span className="brand-logo" aria-hidden="true" />
            StatusDeck
          </Link>
          <QuickSearch />
          <div
            className="account"
            ref={accountRef}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget)) {
                setAccountOpen(false);
              }
            }}
          >
            <button
              ref={accountButtonRef}
              className="account-trigger"
              type="button"
              aria-label="Account menu"
              aria-expanded={accountOpen}
              aria-controls="account-menu"
              onClick={() => setAccountOpen((open) => !open)}
            >
              <UserAvatar name={user.display_name} />
            </button>
            {accountOpen && (
              <nav
                className="account-menu"
                id="account-menu"
                aria-label="Account"
              >
                <Link to="/my-comments" onClick={() => setAccountOpen(false)}>
                  My Comments
                </Link>
                <Link to="/bookmarks" onClick={() => setAccountOpen(false)}>
                  Bookmarks
                </Link>
                <Link to="/profile" onClick={() => setAccountOpen(false)}>
                  Profile
                </Link>
                <button type="button" disabled={loggingOut} onClick={logout}>
                  {loggingOut ? "Logging out…" : "Log out"}
                </button>
              </nav>
            )}
          </div>
        </div>
      </header>
      <aside className="sidebar">
        <p className="sidebar-label">Workspace</p>
        <nav className="primary-nav" aria-label="Primary">
          <Link
            className={location.pathname === "/" ? "active" : ""}
            aria-current={location.pathname === "/" ? "page" : undefined}
            to="/"
          >
            <NavigationIcon name="pulse" />
            Overview
          </Link>
          <Link
            className={location.pathname.startsWith("/catalog") ? "active" : ""}
            aria-current={
              location.pathname.startsWith("/catalog") ? "page" : undefined
            }
            to="/catalog"
          >
            <NavigationIcon name="providers" />
            Providers
          </Link>
          <Link
            className={
              location.pathname.startsWith("/incidents") ? "active" : ""
            }
            aria-current={
              location.pathname.startsWith("/incidents") ? "page" : undefined
            }
            to="/incidents"
          >
            <NavigationIcon name="warning" />
            Incidents
          </Link>
          <Link
            className={
              location.pathname.startsWith("/analytics") ? "active" : ""
            }
            aria-current={
              location.pathname.startsWith("/analytics") ? "page" : undefined
            }
            to="/analytics"
          >
            <NavigationIcon name="chart" />
            Analytics
          </Link>
          <Link
            className={
              location.pathname.startsWith("/notifications") ? "active" : ""
            }
            aria-current={
              location.pathname.startsWith("/notifications")
                ? "page"
                : undefined
            }
            to="/notifications"
          >
            <NavigationIcon name="bell" />
            Notifications
          </Link>
          <Link
            className={location.pathname.startsWith("/system") ? "active" : ""}
            aria-current={
              location.pathname.startsWith("/system") ? "page" : undefined
            }
            to="/system"
          >
            <NavigationIcon name="settings" />
            System
          </Link>
        </nav>
      </aside>
      <main ref={mainRef} className="page" id="main-content" tabIndex={-1}>
        {logoutError && (
          <div className="alert error" role="alert">
            {logoutError}
          </div>
        )}
        <Suspense fallback={<LoadingSkeleton label="Loading page" rows={4} />}>
          <Routes>
            <Route
              path="/login"
              element={<Navigate to={user.preferences.landing_page} replace />}
            />
            <Route
              path="/"
              element={<Dashboard key={user.id} userId={user.id} />}
            />
            <Route path="/catalog" element={<Catalog />} />
            <Route path="/catalog/:id" element={<ProviderDetail />} />
            <Route path="/incidents" element={<Incidents />} />
            <Route path="/incidents/:id" element={<IncidentDetail />} />
            <Route path="/analytics" element={<Analytics />} />
            <Route path="/notifications" element={<Notifications />} />
            <Route
              path="/notifications/summaries/:id"
              element={<NotificationSummary />}
            />
            <Route path="/system" element={<SystemHealth />} />
            <Route path="/my-comments" element={<MyComments />} />
            <Route path="/bookmarks" element={<Bookmarks />} />
            <Route path="/profile" element={<Profile />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </Suspense>
      </main>
    </div>
  );
}

function App() {
  const [user, setUser] = useState<User | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    if (user) setThemePreference(user.preferences.theme);
  }, [user]);
  const session = useQuery({
    queryKey: queryKeys.session,
    queryFn: ({ signal }) => api<User | null>("/api/v1/session", { signal }),
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
  });

  useEffect(() => {
    ensureCsrf()
      .then(() => setReady(true))
      .catch(() => setReady(true));
  }, []);

  useEffect(() => {
    if (session.isSuccess) {
      if (session.data === null) {
        queryClient.removeQueries({
          predicate: (query) => query.queryKey[0] !== queryKeys.session[0],
        });
        queryClient.getMutationCache().clear();
      }
      setUser(session.data);
    }
  }, [session.data, session.isSuccess]);

  async function acceptLogin(profile: User) {
    // A session read started before login must not overwrite the new session.
    await queryClient.cancelQueries();
    queryClient.clear();
    queryClient.setQueryData(queryKeys.session, profile);
    setUser(profile);
  }

  useEffect(() => {
    function unauthorized() {
      queryClient.clear();
      setUser(null);
    }

    window.addEventListener("statusdeck:unauthorized", unauthorized);
    return () =>
      window.removeEventListener("statusdeck:unauthorized", unauthorized);
  }, []);

  if (!ready || session.isLoading)
    return <div className="center-state">Connecting to StatusDeck…</div>;
  if (session.isError && !user)
    return (
      <div className="center-state error-state" role="alert">
        <p>
          StatusDeck could not be reached. Try again after the service recovers.
        </p>
        <button className="button ghost" onClick={() => void session.refetch()}>
          Retry
        </button>
      </div>
    );
  if (!user)
    return (
      <Routes>
        <Route path="*" element={<Login onLogin={acceptLogin} />} />
      </Routes>
    );
  return (
    <ProfileContext.Provider value={user}>
      <Shell user={user} setUser={setUser} />
    </ProfileContext.Provider>
  );
}

export function AppRoot() {
  return (
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <App />
      </ToastProvider>
    </QueryClientProvider>
  );
}
