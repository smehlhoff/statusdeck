import { humanizeIdentifier } from "../../utils/display";

const AWS_UPDATE_STATUSES = new Map([
  ["0", "Resolved"],
  ["1", "Impacted"],
  ["2", "Degraded"],
  ["3", "Disrupted"],
]);

export function incidentUpdateStatus(
  status: string,
  providerSlug?: string,
): string {
  if (providerSlug !== "aws") return humanizeIdentifier(status);
  // Existing AWS updates may contain a JSON-serialized string, e.g. '\"1\"'.
  const code = status.trim().replace(/^"(-?\d+)"$/, "$1");
  const label = AWS_UPDATE_STATUSES.get(code);
  if (label) return label;
  if (/^-?\d+$/.test(code)) return `Unknown AWS status (${code})`;
  return humanizeIdentifier(status);
}
