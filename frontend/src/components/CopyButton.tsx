import { useEffect, useState } from "react";
import { copyText } from "../utils/clipboard";
import { useToast } from "./toastContext";

export function CopyButton({
  value,
  label = "Copy",
  ariaLabel = label,
  className = "button ghost",
  errorMessage,
}: {
  value: string;
  label?: string;
  ariaLabel?: string;
  className?: string;
  errorMessage: string;
}) {
  const { notify } = useToast();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(false), 2000);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  async function copy() {
    try {
      await copyText(value);
      setCopied(true);
    } catch {
      setCopied(false);
      notify(errorMessage, "error");
    }
  }

  return (
    <button
      className={className}
      type="button"
      aria-label={copied ? "Copied" : ariaLabel}
      aria-live="polite"
      onClick={() => void copy()}
    >
      {copied ? "Copied" : label}
    </button>
  );
}
