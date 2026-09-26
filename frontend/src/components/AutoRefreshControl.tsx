import { formatDuration } from "../utils/display";

interface AutoRefreshControlProps {
  interval: number;
  onChange: () => void;
}

export function AutoRefreshControl({
  interval,
  onChange,
}: AutoRefreshControlProps) {
  const enabled = interval > 0;
  return (
    <button
      className="button ghost refresh-toggle"
      type="button"
      data-state={enabled ? "on" : "off"}
      aria-label="Change automatic refresh interval"
      title={
        enabled
          ? "Auto refresh is active. Select to change the interval."
          : "Auto refresh is off. Select to choose an interval."
      }
      onClick={onChange}
    >
      {enabled ? `Refresh ${formatDuration(interval / 1_000)}` : "Refresh off"}
    </button>
  );
}
