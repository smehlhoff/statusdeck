import type { DisplayPreferences } from "../../api/types";
import { useSyncExternalStore } from "react";

type Theme = DisplayPreferences["theme"];
const STORAGE_KEY = "statusdeck:theme";
let preference: Theme = "light";

function subscribe(listener: () => void) {
  window.addEventListener("statusdeck:theme-changed", listener);
  return () => window.removeEventListener("statusdeck:theme-changed", listener);
}

function resolvedTheme() {
  return document.documentElement.dataset.theme ?? "light";
}

export function useResolvedTheme() {
  return useSyncExternalStore(subscribe, resolvedTheme);
}

function applyTheme() {
  const dark =
    preference === "dark" ||
    (preference === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);
  const resolved = dark ? "dark" : "light";
  if (document.documentElement.dataset.theme === resolved) return;
  document.documentElement.dataset.theme = resolved;
  window.dispatchEvent(new Event("statusdeck:theme-changed"));
}

export function setThemePreference(theme: Theme) {
  preference = theme;
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Account preferences still work when browser storage is unavailable.
  }
  applyTheme();
}

export function initializeTheme() {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "light" || stored === "dark" || stored === "system")
      preference = stored;
  } catch {
    // Use light until the account preference loads.
  }
  applyTheme();
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", applyTheme);
}
