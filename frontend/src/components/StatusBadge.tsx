import { humanizeIdentifier } from "../utils/display";

interface StatusBadgeProps {
  value: string;
  tone?: string;
  label?: string;
}

export function StatusBadge({ value, tone = value, label }: StatusBadgeProps) {
  return (
    <span className={`badge ${tone}`}>
      {label ?? humanizeIdentifier(value)}
    </span>
  );
}
