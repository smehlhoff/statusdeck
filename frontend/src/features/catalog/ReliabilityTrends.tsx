import { useEffect, useMemo, useRef } from "react";
import Chart from "chart.js/auto";
import type { ReliabilityDay } from "../../api/types";
import { formatDuration, resolvedTimeZone } from "../../utils/display";
import { useProfile } from "../profile/profileContext";
import { useResolvedTheme } from "../profile/theme";

function monthlyBuckets(days: ReliabilityDay[]) {
  const buckets: Array<{
    month: string;
    first: string;
    last: string;
    major: number;
    minor: number;
    seconds: number;
    unknownDurations: number;
    recorded: number;
    days: number;
  }> = [];
  for (const day of days) {
    const month = day.date.slice(0, 7);
    let bucket = buckets.at(-1);
    if (!bucket || bucket.month !== month) {
      bucket = {
        month,
        first: day.date,
        last: day.date,
        major: 0,
        minor: 0,
        seconds: 0,
        unknownDurations: 0,
        recorded: 0,
        days: 0,
      };
      buckets.push(bucket);
    }
    bucket.last = day.date;
    bucket.major += day.started_major_count;
    bucket.minor += day.started_minor_count;
    bucket.seconds += day.affected_seconds;
    bucket.unknownDurations += day.unknown_incident_duration_count;
    bucket.recorded += Number(day.status !== "unknown");
    bucket.days += 1;
  }
  return buckets;
}

export function ReliabilityTrends({ days }: { days: ReliabilityDay[] }) {
  const { preferences } = useProfile();
  const timeZone = resolvedTimeZone(preferences);
  const theme = useResolvedTheme();
  const frequencyRef = useRef<HTMLCanvasElement>(null);
  const durationRef = useRef<HTMLCanvasElement>(null);
  const buckets = useMemo(() => monthlyBuckets(days), [days]);
  const count = buckets.reduce(
    (sum, bucket) => sum + bucket.major + bucket.minor,
    0,
  );
  const unknownDurations = days.reduce(
    (sum, day) => sum + day.unknown_incident_duration_count,
    0,
  );
  const seconds = buckets.reduce((sum, bucket) => sum + bucket.seconds, 0);
  const durationLabel =
    seconds === 0 && unknownDurations > 0 ? "Unknown" : formatDuration(seconds);
  const hasRecords = days.some(
    (day) =>
      day.status !== "unknown" ||
      day.started_major_count + day.started_minor_count > 0,
  );

  useEffect(() => {
    if (!frequencyRef.current || !durationRef.current) return;
    const styles = getComputedStyle(frequencyRef.current);
    const color = (name: string) => styles.getPropertyValue(name).trim();
    const labels = buckets.map((bucket) =>
      new Date(`${bucket.month}-01T00:00:00Z`).toLocaleDateString(undefined, {
        month: "short",
        year: "2-digit",
        timeZone: "UTC",
      }),
    );
    const charts = (["frequency", "duration"] as const).map((kind) => {
      const canvas =
        kind === "frequency" ? frequencyRef.current : durationRef.current;
      if (!canvas) return null;
      return new Chart(canvas, {
        type: "bar",
        data: {
          labels,
          datasets:
            kind === "frequency"
              ? [
                  {
                    label: "Minor impact",
                    data: buckets.map((b) =>
                      b.recorded > 0 || b.minor > 0 || b.major > 0
                        ? b.minor
                        : null,
                    ),
                    backgroundColor: color("--rp-minor"),
                    borderRadius: 3,
                  },
                  {
                    label: "Major impact",
                    data: buckets.map((b) =>
                      b.recorded > 0 || b.minor > 0 || b.major > 0
                        ? b.major
                        : null,
                    ),
                    backgroundColor: color("--rp-major"),
                    borderRadius: 3,
                  },
                ]
              : [
                  {
                    label: "Reported impact",
                    data: buckets.map((b) =>
                      b.recorded > 0 &&
                      (b.seconds > 0 || b.unknownDurations === 0)
                        ? b.seconds / 3600
                        : null,
                    ),
                    backgroundColor: color("--accent"),
                    borderRadius: 3,
                  },
                ],
        },
        options: {
          responsive: true,
          maintainAspectRatio: false,
          animation: false,
          interaction: { mode: "index", intersect: false },
          plugins: {
            legend: { display: false },
            tooltip: {
              callbacks: {
                title: (items) => {
                  const b = buckets[items[0]?.dataIndex ?? -1];
                  return b ? `${b.first} – ${b.last} · ${timeZone}` : "";
                },
                label: (context) =>
                  `${context.dataset.label}: ${kind === "frequency" ? context.raw : formatDuration(Number(context.raw) * 3600)}`,
                footer: (items) => {
                  const b = buckets[items[0]?.dataIndex ?? -1];
                  return b
                    ? `${b.recorded} / ${b.days} days with records · History completeness unverified`
                    : "";
                },
              },
            },
          },
          scales: {
            x: {
              stacked: true,
              grid: { display: false },
              border: { display: false },
              ticks: {
                color: color("--muted"),
                maxRotation: 0,
                autoSkip: true,
                font: { size: 10 },
              },
            },
            y: {
              stacked: true,
              beginAtZero: true,
              suggestedMax: 1,
              border: { display: false },
              grid: { color: color("--chart-grid") },
              title: {
                display: true,
                text: kind === "frequency" ? "Incidents" : "Hours",
                color: color("--muted"),
              },
              ticks: {
                color: color("--muted"),
                precision: kind === "frequency" ? 0 : undefined,
                font: { size: 10 },
              },
            },
          },
        },
      });
    });
    return () => {
      for (const chart of charts) chart?.destroy();
    };
  }, [buckets, theme, timeZone]);

  return (
    <div className="rp-trends">
      <section
        className="card rp-trend-card"
        aria-labelledby="rp-frequency-heading"
      >
        <h2 id="rp-frequency-heading">Incident frequency</h2>
        <p className="section-description">
          Recorded incidents by known date · Maintenance excluded
        </p>
        <div className="rp-trend-total">
          <strong>{hasRecords ? count : "—"}</strong>
          <span>incidents recorded in this period</span>
        </div>
        <div className="rp-trend-canvas">
          <canvas
            ref={frequencyRef}
            role="img"
            aria-label={`Monthly recorded incident frequency: ${count}. History is incomplete.`}
          />
        </div>
        <div className="rp-legend rp-frequency-legend">
          <span>
            <i className="rp-key rp-minor" />
            Minor impact
          </span>
          <span>
            <i className="rp-key rp-major" />
            Major impact
          </span>
        </div>
        {count === 0 && (
          <p className="rp-trend-note">
            {hasRecords
              ? "No incidents recorded in this period."
              : "No incident history is available for this period."}
          </p>
        )}
      </section>
      <section
        className="card rp-trend-card"
        aria-labelledby="rp-duration-heading"
      >
        <h2 id="rp-duration-heading">Reported impact duration</h2>
        <p className="section-description">
          Overlapping incidents counted once · Maintenance excluded
        </p>
        <div className="rp-trend-total">
          <strong>{hasRecords ? durationLabel : "—"}</strong>
          <span>reported impact in this period</span>
        </div>
        <div className="rp-trend-canvas">
          <canvas
            ref={durationRef}
            role="img"
            aria-label={`Monthly reported impact duration: ${hasRecords ? durationLabel : "No history available"}.`}
          />
        </div>
        {seconds === 0 && (
          <p className="rp-trend-note">
            {hasRecords
              ? "No known incident duration is recorded in this period."
              : "No incident history is available for this period."}
          </p>
        )}
      </section>
    </div>
  );
}
