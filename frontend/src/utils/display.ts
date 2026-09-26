import type { DisplayPreferences } from "../api/types";

const locales = {
  locale: undefined,
  day_first: "en-GB",
  month_first: "en-US",
  iso: "en-CA",
};

export function resolvedTimeZone(preferences?: DisplayPreferences): string {
  const value = preferences?.time_zone;
  if (!value || value === "browser")
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  try {
    new Intl.DateTimeFormat(undefined, { timeZone: value });
    return value;
  } catch {
    return "UTC";
  }
}

export function formatDateTime(
  value: string,
  preferences?: DisplayPreferences,
): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const locale = locales[preferences?.date_format ?? "locale"];
  const options: Intl.DateTimeFormatOptions = {
    year: "numeric",
    month:
      !preferences || preferences.date_format === "locale"
        ? "short"
        : "2-digit",
    day:
      !preferences || preferences.date_format === "locale"
        ? "numeric"
        : "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    timeZoneName: "short",
  };
  if (preferences?.time_format === "twelve_hour") options.hour12 = true;
  if (preferences?.time_format === "twenty_four_hour")
    options.hourCycle = "h23";
  options.timeZone = resolvedTimeZone(preferences);
  return new Intl.DateTimeFormat(locale, options).format(date);
}

export function formatDate(
  value: string,
  preferences?: DisplayPreferences,
  timeZone = resolvedTimeZone(preferences),
): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(
    locales[preferences?.date_format ?? "locale"],
    {
      year: "numeric",
      month:
        !preferences || preferences.date_format === "locale"
          ? "short"
          : "2-digit",
      day:
        !preferences || preferences.date_format === "locale"
          ? "numeric"
          : "2-digit",
      timeZone,
    },
  ).format(date);
}

function dateTimeParts(value: Date, timeZone: string) {
  return Object.fromEntries(
    new Intl.DateTimeFormat("en-CA", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
      timeZone,
    })
      .formatToParts(value)
      .filter((part) => part.type !== "literal")
      .map((part) => [part.type, part.value]),
  );
}

export function dateTimeInputValue(
  value: string | null,
  preferences?: DisplayPreferences,
): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const parts = dateTimeParts(date, resolvedTimeZone(preferences));
  return `${parts.year}-${parts.month}-${parts.day}T${parts.hour}:${parts.minute}`;
}

export function dateTimeInputIso(
  value: string,
  preferences?: DisplayPreferences,
): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(value);
  if (!match) return value;
  const desired = Date.UTC(
    ...(match
      .slice(1)
      .map(Number)
      .map((part, index) => (index === 1 ? part - 1 : part)) as [
      number,
      number,
      number,
      number,
      number,
    ]),
  );
  const timeZone = resolvedTimeZone(preferences);
  let timestamp = desired;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const parts = dateTimeParts(new Date(timestamp), timeZone);
    const rendered = Date.UTC(
      Number(parts.year),
      Number(parts.month) - 1,
      Number(parts.day),
      Number(parts.hour),
      Number(parts.minute),
    );
    timestamp += desired - rendered;
  }
  return new Date(timestamp).toISOString();
}

/** Format seconds using at most two nonzero units; missing values display as a dash. */
export function formatDuration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return "—";
  if (seconds <= 0) return "0s";
  const milliseconds = Math.max(1, Math.round(seconds * 1_000));
  if (milliseconds < 1_000) return `${milliseconds}ms`;

  let remaining = Math.round(seconds);
  const parts: string[] = [];
  for (const [unit, size] of [
    ["d", 86_400],
    ["h", 3_600],
    ["m", 60],
    ["s", 1],
  ] as const) {
    const amount = Math.floor(remaining / size);
    if (amount > 0) parts.push(`${amount}${unit}`);
    remaining %= size;
    if (parts.length === 2) break;
  }
  return parts.join(" ");
}

export function humanizeIdentifier(value: string): string {
  return value.replaceAll("_", " ");
}

export function formatProviderPhase(value: string): string {
  const phase = humanizeIdentifier(value).trim().toLowerCase();
  return phase.charAt(0).toUpperCase() + phase.slice(1);
}

export function maintenanceTiming(
  startAt: string | null,
  endAt: string | null,
  now = Date.now(),
): "Upcoming" | "Scheduled window" | "Completion unconfirmed" | "Scheduled" {
  const start = startAt ? Date.parse(startAt) : Number.NaN;
  const end = endAt ? Date.parse(endAt) : Number.NaN;

  if (Number.isFinite(start) && now < start) return "Upcoming";
  if (Number.isFinite(start) && (!Number.isFinite(end) || now < end)) {
    return "Scheduled window";
  }
  if (Number.isFinite(end) && now >= end) return "Completion unconfirmed";
  return "Scheduled";
}

export function providerText(value: string): string {
  const withLineBreaks = value.replace(/<br\s*\/?>|<\/p>/gi, "\n");
  const document = new DOMParser().parseFromString(withLineBreaks, "text/html");
  return (document.body.textContent ?? value).replace(/\n{3,}/g, "\n\n").trim();
}
