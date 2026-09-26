import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import type { QuietHours } from "../../api/types";

const TIMEZONES =
  typeof Intl.supportedValuesOf === "function"
    ? Intl.supportedValuesOf("timeZone")
    : [
        "America/New_York",
        "America/Chicago",
        "America/Denver",
        "America/Los_Angeles",
        "Europe/London",
        "Europe/Paris",
        "Asia/Tokyo",
        "Australia/Sydney",
      ];

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

export function QuietHoursFields({
  value,
  onChange,
  disabled,
}: {
  value: QuietHours;
  onChange: (value: QuietHours) => void;
  disabled: boolean;
}) {
  const timezones = [
    "UTC",
    ...Array.from(new Set([...TIMEZONES, value.timezone]))
      .filter((zone) => zone !== "UTC")
      .sort(),
  ];
  const preview = useQuery({
    queryKey: ["quiet-hours-preview", value],
    queryFn: ({ signal }) =>
      api<{ quiet_until: string | null }>("/api/v1/quiet-hours/preview", {
        method: "POST",
        body: JSON.stringify(value),
        signal,
      }),
    enabled:
      value.enabled &&
      value.days.length > 0 &&
      Boolean(value.start && value.end && value.timezone),
    retry: false,
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
  });
  let until: string | undefined;
  if (preview.data?.quiet_until) {
    try {
      until = new Intl.DateTimeFormat(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
        timeZone: value.timezone,
      }).format(new Date(preview.data.quiet_until));
    } catch {
      /* The API reports invalid timezone input below. */
    }
  }
  return (
    <fieldset className="quiet-hours" disabled={disabled}>
      <legend>Quiet hours</legend>
      <label className="check">
        <input
          type="checkbox"
          checked={value.enabled}
          onChange={(event) =>
            onChange({ ...value, enabled: event.target.checked })
          }
        />
        Enable quiet hours
      </label>
      <p className="fieldset-help">
        Pause notifications during your schedule and receive a summary
        afterward. Monitoring stays on.
      </p>
      {value.enabled && (
        <>
          <div className="quiet-hours-schedule">
            <fieldset className="quiet-hours-days">
              <legend>Start days</legend>
              <div className="quiet-hours-day-options">
                {DAYS.map((day, index) => (
                  <label className="check" key={day}>
                    <input
                      type="checkbox"
                      checked={value.days.includes(index + 1)}
                      onChange={() =>
                        onChange({
                          ...value,
                          days: value.days.includes(index + 1)
                            ? value.days.filter((d) => d !== index + 1)
                            : [...value.days, index + 1].sort(),
                        })
                      }
                    />
                    {day}
                  </label>
                ))}
              </div>
              <p className="fieldset-help">
                Overnight schedules end the following day.
              </p>
              {!value.days.length && (
                <p className="alert error">Choose at least one day.</p>
              )}
            </fieldset>
            <div className="quiet-hours-times">
              <label>
                Start time
                <input
                  required
                  type="time"
                  value={value.start}
                  onChange={(event) =>
                    onChange({ ...value, start: event.target.value })
                  }
                />
              </label>
              <label>
                End time
                <input
                  required
                  type="time"
                  value={value.end}
                  onChange={(event) =>
                    onChange({ ...value, end: event.target.value })
                  }
                />
              </label>
              <label>
                Timezone
                <select
                  required
                  value={value.timezone}
                  onChange={(event) =>
                    onChange({ ...value, timezone: event.target.value })
                  }
                >
                  {timezones.map((zone) => (
                    <option key={zone} value={zone}>
                      {zone}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <p className="fieldset-help">
              Times adjust automatically for daylight saving in the selected
              timezone.
            </p>
          </div>
          <div className="quiet-hours-exception">
            <label className="check">
              <input
                type="checkbox"
                checked={value.critical_override}
                onChange={(event) =>
                  onChange({
                    ...value,
                    critical_override: event.target.checked,
                  })
                }
              />
              Allow urgent alerts during quiet hours
            </label>
            <p className="fieldset-help">
              Critical incidents and major outages bypass this schedule.
            </p>
          </div>
          {preview.isError && (
            <p className="alert error" role="alert">
              {preview.error.message}
            </p>
          )}
          {preview.isFetching && (
            <p className="fieldset-help" role="status">
              Checking schedule…
            </p>
          )}
          {!preview.isFetching && preview.isSuccess && (
            <p className="fieldset-help quiet-hours-status" role="status">
              <svg
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                aria-hidden="true"
              >
                <circle cx="12" cy="12" r="9" />
                <path d="M12 7v6m0 4h.01" />
              </svg>
              <span>
                <strong>Schedule preview</strong>
                {until
                  ? `Notifications would be held until ${until} (${value.timezone}).${value.critical_override ? " Critical alerts bypass quiet hours." : ""}`
                  : "Outside quiet hours right now. Notifications would be sent as usual."}
              </span>
            </p>
          )}
          <details className="quiet-hours-details">
            <summary>How summaries work</summary>
            <p className="fieldset-help">
              Each channel receives one summary of held events for this rule,
              showing whether incidents are still active. Test notifications
              always bypass quiet hours.
            </p>
          </details>
        </>
      )}
    </fieldset>
  );
}
