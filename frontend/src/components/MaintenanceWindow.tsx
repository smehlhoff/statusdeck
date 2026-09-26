import { RelativeDateTime } from "./RelativeDateTime";

export function MaintenanceWindow({
  startAt,
  endAt,
  fallbackAt,
  className,
}: {
  startAt: string | null;
  endAt: string | null;
  fallbackAt?: string;
  className?: string;
}) {
  if (startAt && endAt) {
    return (
      <span className={className}>
        Scheduled <RelativeDateTime value={startAt} />
        {" – "}
        <RelativeDateTime value={endAt} />
      </span>
    );
  }
  if (startAt) {
    return (
      <span className={className}>
        Starts <RelativeDateTime value={startAt} />
      </span>
    );
  }
  if (endAt) {
    return (
      <span className={className}>
        Scheduled until <RelativeDateTime value={endAt} />
      </span>
    );
  }
  if (fallbackAt) {
    return (
      <span className={className}>
        Last observed <RelativeDateTime value={fallbackAt} />
      </span>
    );
  }
  return null;
}
