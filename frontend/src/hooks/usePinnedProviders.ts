import { useEffect, useState } from "react";

function readPins(key: string): Set<string> {
  try {
    const stored: unknown = JSON.parse(
      window.localStorage.getItem(key) ?? "[]",
    );
    if (Array.isArray(stored) && stored.every((id) => typeof id === "string")) {
      return new Set(stored);
    }
  } catch {
    // Invalid or unavailable storage must not prevent the dashboard from loading.
  }
  return new Set();
}

export function usePinnedProviders(userId: string) {
  const storageKey = `statusdeck-pinned-providers:${userId}`;
  const [pinnedIds, setPinnedIds] = useState(() => readPins(storageKey));
  const [storageError, setStorageError] = useState(false);

  useEffect(() => {
    function syncPins(event: StorageEvent) {
      if (
        event.storageArea === window.localStorage &&
        (event.key === storageKey || event.key === null)
      ) {
        setPinnedIds(readPins(storageKey));
      }
    }

    window.addEventListener("storage", syncPins);
    return () => window.removeEventListener("storage", syncPins);
  }, [storageKey]);

  function togglePin(id: string) {
    const next = new Set(pinnedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setPinnedIds(next);
    try {
      window.localStorage.setItem(storageKey, JSON.stringify([...next]));
      setStorageError(false);
    } catch {
      setStorageError(true);
    }
  }

  return { pinnedIds, togglePin, storageError };
}
