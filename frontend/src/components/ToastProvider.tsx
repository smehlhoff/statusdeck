import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { ToastContext, type ToastTone } from "./toastContext";

interface Toast {
  id: number;
  message: string;
  tone: ToastTone;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(0);
  const dismissTimers = useRef(new Map<number, number>());

  const dismiss = useCallback((id: number) => {
    const timer = dismissTimers.current.get(id);
    if (timer !== undefined) window.clearTimeout(timer);
    dismissTimers.current.delete(id);
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const notify = useCallback(
    (message: string, tone: ToastTone = "success") => {
      const id = ++nextId.current;
      setToasts((current) => [...current, { id, message, tone }]);
      dismissTimers.current.set(
        id,
        window.setTimeout(() => dismiss(id), 5_000),
      );
    },
    [dismiss],
  );

  useEffect(
    () => () => {
      for (const timer of dismissTimers.current.values()) {
        window.clearTimeout(timer);
      }
      dismissTimers.current.clear();
    },
    [],
  );

  const value = useMemo(() => ({ notify }), [notify]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-region" aria-live="polite" aria-atomic="false">
        {toasts.map((toast) => (
          <div className={`toast ${toast.tone}`} key={toast.id}>
            <span>{toast.message}</span>
            <button
              type="button"
              aria-label="Dismiss notification"
              onClick={() => dismiss(toast.id)}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
