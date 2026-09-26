import { useToast } from "../../components/toastContext";

async function copyText(value: string): Promise<void> {
  if (navigator.clipboard) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // Fall back for browsers that expose the API but block it on LAN HTTP.
    }
  }

  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.readOnly = true;
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.append(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  textarea.remove();
  if (!copied) throw new Error("copy failed");
}

export function CopyIncidentLinkButton({ incidentId }: { incidentId: string }) {
  const { notify } = useToast();

  async function copyIncidentLink() {
    try {
      await copyText(
        new URL(`/incidents/${incidentId}`, window.location.origin).toString(),
      );
      notify("Incident link copied.");
    } catch {
      notify("Could not copy the incident link.", "error");
    }
  }

  return (
    <button
      className="button ghost"
      type="button"
      onClick={() => void copyIncidentLink()}
    >
      Copy link
    </button>
  );
}
