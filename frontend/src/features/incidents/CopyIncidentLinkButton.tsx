import { useToast } from "../../components/toastContext";
import { copyText } from "../../utils/clipboard";

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
