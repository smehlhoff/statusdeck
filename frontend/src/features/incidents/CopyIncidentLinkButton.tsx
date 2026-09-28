import { CopyButton } from "../../components/CopyButton";

export function CopyIncidentLinkButton({ incidentId }: { incidentId: string }) {
  return (
    <CopyButton
      value={new URL(
        `/incidents/${incidentId}`,
        window.location.origin,
      ).toString()}
      label="Copy link"
      errorMessage="Could not copy the incident link."
    />
  );
}
