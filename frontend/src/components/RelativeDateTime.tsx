import { useProfile } from "../features/profile/profileContext";
import { formatDateTime } from "../utils/display";

const RELATIVE_TIME_FORMATTER = new Intl.RelativeTimeFormat(undefined, {
  numeric: "auto",
});

function relativeDateTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return value;
  const differenceSeconds = Math.round((timestamp - Date.now()) / 1_000);
  const absoluteSeconds = Math.abs(differenceSeconds);

  if (absoluteSeconds < 60)
    return RELATIVE_TIME_FORMATTER.format(differenceSeconds, "second");
  const minutes = Math.round(differenceSeconds / 60);
  if (Math.abs(minutes) < 60)
    return RELATIVE_TIME_FORMATTER.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24)
    return RELATIVE_TIME_FORMATTER.format(hours, "hour");
  const days = Math.round(hours / 24);
  if (Math.abs(days) < 30) return RELATIVE_TIME_FORMATTER.format(days, "day");
  const months = Math.round(days / 30);
  if (Math.abs(months) < 12)
    return RELATIVE_TIME_FORMATTER.format(months, "month");
  return RELATIVE_TIME_FORMATTER.format(Math.round(days / 365), "year");
}

export function RelativeDateTime({
  value,
  fallback = "Not yet",
  prefix,
}: {
  value: string | null;
  fallback?: string;
  prefix?: string;
}) {
  const { preferences } = useProfile();
  if (!value) return <>{fallback}</>;
  const absolute = formatDateTime(value, preferences);
  return (
    <time dateTime={value} title={absolute}>
      {prefix}
      {preferences.timestamp_format === "relative"
        ? relativeDateTime(value)
        : absolute}
    </time>
  );
}
