import { SearchableSelect } from "../../components/SearchableSelect";
import { useId, useState, type CSSProperties } from "react";
import { Link } from "react-router-dom";
import type {
  Component,
  ProviderReliability,
  ReliabilityDay,
} from "../../api/types";
import {
  formatDate,
  formatDuration,
  resolvedTimeZone,
} from "../../utils/display";
import { useProfile } from "../profile/profileContext";
import "./reliability-preview.css";
import { ReliabilityTrends } from "./ReliabilityTrends";

const LABELS = {
  healthy: "No reported impact",
  minor: "Minor impact",
  major: "Major impact",
  maintenance: "Maintenance only",
  unknown: "No history",
};
const SYMBOLS = {
  healthy: "",
  minor: "·",
  major: "!",
  maintenance: "−",
  unknown: "",
};

function dayDescription(day: ReliabilityDay) {
  if (day.unknown_duration_count > 0)
    return `${day.incident_count} incidents · ${day.unknown_duration_count} event durations unknown. Unknown durations are excluded from affected time.`;
  if (day.status === "unknown")
    return "No successful poll or matching incident record is available for this day. Provider health cannot be inferred.";
  if (day.incident_count > 0)
    return `${day.incident_count} incident${day.incident_count === 1 ? "" : "s"} · ${formatDuration(day.affected_seconds)} of reported impact`;
  if (day.maintenance_count > 0)
    return `${day.maintenance_count} maintenance event${day.maintenance_count === 1 ? "" : "s"} · Excluded from incident impact`;
  return "A successful poll was recorded, with no incident impact in the available history. This does not guarantee continuous coverage.";
}

export function ReliabilityCharts({
  data,
  components,
  period,
  onPeriod,
  component,
  onComponent,
  providerId,
}: {
  data: ProviderReliability;
  components: Array<Pick<Component, "id" | "name" | "group">>;
  period: number;
  onPeriod: (value: number) => void;
  component: string;
  onComponent: (value: string) => void;
  providerId?: string;
}) {
  const { preferences } = useProfile();
  const timeZone = resolvedTimeZone(preferences);
  const dateLabel = (value: string) =>
    formatDate(`${value}T00:00:00Z`, preferences, "UTC");
  const historyHelpId = useId();
  const [selected, setSelected] = useState<string | null>(null);
  const days = data.days;
  const first = days[0];
  const last = days.at(-1);
  const affected = days.filter((day) => day.incident_count > 0);
  const active = days.find((day) => day.date === selected) ?? last;
  if (!first || !last || !active)
    return (
      <section className="card provider-detail-section">
        <h2>Reliability history</h2>
        <p className="muted">No history is available.</p>
      </section>
    );
  const offset = new Date(`${first.date}T00:00:00Z`).getUTCDay();
  const weeks = Math.ceil((days.length + offset) / 7);
  const known = days.filter((day) => day.status !== "unknown").length;
  const unknownDurations = days.reduce(
    (sum, day) => sum + day.unknown_duration_count,
    0,
  );
  const affectedSeconds = days.reduce(
    (sum, day) => sum + day.affected_seconds,
    0,
  );
  let affectedDuration = "—";
  if (known > 0) {
    affectedDuration =
      affectedSeconds === 0 &&
      days.some((day) => day.unknown_incident_duration_count > 0)
        ? "Unknown"
        : formatDuration(affectedSeconds);
  }
  const months = days.flatMap((day, index) => {
    const date = new Date(`${day.date}T00:00:00Z`);
    return index === 0 || date.getUTCDate() === 1
      ? [
          {
            name: date.toLocaleDateString(undefined, {
              month: "short",
              timeZone: "UTC",
            }),
            column: Math.floor((index + offset) / 7) + 1,
          },
        ]
      : [];
  });
  const incidentParams = new URLSearchParams({
    provider_id: providerId ?? "",
    scope: "provider",
    from: active.from,
    to: active.to,
  });
  if (component !== "all") incidentParams.set("component_id", component);
  return (
    <div className="reliability-preview provider-reliability">
      <section className="card rp-history" aria-labelledby="rp-history-heading">
        <div className="section-heading rp-heading">
          <div>
            <div className="field-help-label rp-history-help">
              <h2 id="rp-history-heading">Reliability history</h2>
              {(!data.coverage_complete || unknownDurations > 0) && (
                <>
                  <button
                    className="field-help-button field-warning-button"
                    type="button"
                    aria-label="Reliability history warnings"
                    aria-describedby={historyHelpId}
                  >
                    <svg
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      aria-hidden="true"
                    >
                      <path d="M12 3 2 21h20L12 3Z" />
                      <path d="M12 9v5m0 3h.01" />
                    </svg>
                  </button>
                  <div
                    className="field-help-tooltip"
                    id={historyHelpId}
                    role="tooltip"
                  >
                    <p>
                      Duration runs from impact start (or first report) to
                      resolution, or now. Affected time counts overlaps once
                      within the displayed {timeZone} days and excludes
                      maintenance.
                    </p>
                    {!data.coverage_complete && (
                      <p>
                        History is incomplete; empty periods may have unrecorded
                        incidents.
                        {data.history_available_from &&
                          ` Records from ${formatDate(data.history_available_from, preferences)}.`}
                        {data.history_refreshed_at &&
                          ` Last checked ${formatDate(data.history_refreshed_at, preferences)}.`}
                      </p>
                    )}
                    {unknownDurations > 0 && (
                      <p>
                        {unknownDurations} event durations are unknown (missing
                        or inconsistent timestamps). Included in counts;
                        excluded from affected time.
                      </p>
                    )}
                  </div>
                </>
              )}
            </div>
            <p className="section-description">
              Select a day to explore its reported impact.
            </p>
          </div>
          <div className="rp-controls">
            <SearchableSelect
              value={component}
              onChange={onComponent}
              label="Component"
              hideLabel
              searchLabel="Search provider components"
              placeholder="Search components"
              emptyMessage="No matching components."
              options={[
                { value: "all", label: "All provider components" },
                ...components
                  .map((item) => ({
                    value: item.id,
                    label: `${item.name}${item.group ? ` — ${item.group}` : ""}`,
                  }))
                  .sort((a, b) =>
                    a.label.localeCompare(b.label, undefined, {
                      sensitivity: "base",
                      numeric: true,
                    }),
                  ),
              ]}
            />
            <div className="rp-periods" role="group" aria-label="History range">
              {[90, 180, 365].map((value) => (
                <button
                  key={value}
                  type="button"
                  aria-pressed={period === value}
                  onClick={() => onPeriod(value)}
                >
                  {value} days
                </button>
              ))}
            </div>
          </div>
        </div>
        <div className="rp-metrics">
          <div>
            <span>Recorded days without known incident impact</span>
            <strong>
              {known - affected.length}
              <small> / {known} with records</small>
            </strong>
            <p>Includes maintenance-only days</p>
          </div>
          <div>
            <span>Reported affected time</span>
            <strong>{affectedDuration}</strong>
            <p>Known durations only; overlapping incidents counted once</p>
          </div>
          <div>
            <span>Days with incidents</span>
            <strong>{affected.length}</strong>
            <p>
              {days.filter((day) => day.status === "major").length} with major
              impact
            </p>
          </div>
          <div>
            <span>Days with records</span>
            <strong>
              {known}
              <small> / {days.length} days</small>
            </strong>
            <p>Missing records are never counted as healthy</p>
          </div>
        </div>
        <div
          className="rp-calendar-scroll"
          role="region"
          aria-label="Daily reliability calendar"
          tabIndex={0}
        >
          <div
            className="rp-calendar"
            style={
              {
                "--weeks": weeks,
              } as CSSProperties
            }
          >
            <div className="rp-months">
              {months.map((month, index) => (
                <span key={index} style={{ gridColumn: month.column }}>
                  {month.name}
                </span>
              ))}
            </div>
            <div className="rp-day-names" aria-hidden="true">
              {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((name) => (
                <span key={name}>{name}</span>
              ))}
            </div>
            <div className="rp-squares">
              {Array.from({ length: offset }, (_, index) => (
                <span key={`empty-${index}`} />
              ))}
              {days.map((day, index) => (
                <button
                  key={day.date}
                  type="button"
                  className={`rp-day rp-${day.status}`}
                  aria-label={`${dateLabel(day.date)}: ${LABELS[day.status]}. ${dayDescription(day)}`}
                  aria-pressed={active.date === day.date}
                  tabIndex={active.date === day.date ? 0 : -1}
                  title={`${dateLabel(day.date)} · ${LABELS[day.status]}`}
                  onClick={() => setSelected(day.date)}
                  onKeyDown={(event) => {
                    const step = {
                      ArrowRight: 7,
                      ArrowLeft: -7,
                      ArrowDown: 1,
                      ArrowUp: -1,
                    }[event.key];
                    if (step === undefined) return;
                    event.preventDefault();
                    const next = Math.max(
                      0,
                      Math.min(days.length - 1, index + step),
                    );
                    const nextDay = days[next];
                    if (nextDay) setSelected(nextDay.date);
                    const buttons =
                      event.currentTarget.parentElement?.querySelectorAll(
                        "button",
                      );
                    buttons?.item(next)?.focus();
                  }}
                >
                  <span aria-hidden="true">{SYMBOLS[day.status]}</span>
                </button>
              ))}
            </div>
          </div>
        </div>
        <div className="rp-calendar-footer">
          <span>
            {dateLabel(first.date)} – {dateLabel(last.date)} · {timeZone}
          </span>
          <div className="rp-legend">
            {Object.entries(LABELS).map(([status, label]) => (
              <span key={status}>
                <i className={`rp-key rp-${status}`} />
                {label}
              </span>
            ))}
          </div>
        </div>
        <div className="rp-day-detail" aria-live="polite" aria-atomic="true">
          <div>
            <span className="eyebrow">Selected day</span>
            <h3>{dateLabel(active.date)}</h3>
            <span
              className={`badge ${["major", "minor"].includes(active.status) ? active.status : "neutral"}`}
            >
              {LABELS[active.status]}
            </span>
          </div>
          <div>
            <strong>{LABELS[active.status]}</strong>
            <p>{dayDescription(active)}</p>
            {active.incident_count > 0 && active.maintenance_count > 0 && (
              <p>
                {active.maintenance_count} maintenance events also reported;
                excluded from affected time.
              </p>
            )}
            {providerId && (
              <Link
                className="rp-incident-link"
                to={`/incidents?${incidentParams}`}
              >
                Browse incidents for this day →
              </Link>
            )}
          </div>
        </div>
      </section>
      <ReliabilityTrends days={days} />
    </div>
  );
}
