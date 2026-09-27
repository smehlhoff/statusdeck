import { useId } from "react";

export function FieldHelp({ label, help }: { label: string; help: string }) {
  const tooltipId = useId();
  return (
    <>
      <button
        className="field-help-button"
        type="button"
        aria-label={`About ${label}`}
        aria-describedby={tooltipId}
        onClick={(event) => event.currentTarget.focus()}
        onKeyDown={(event) => {
          if (event.key === "Escape") event.currentTarget.blur();
        }}
      >
        ?
      </button>
      <span className="field-help-tooltip" id={tooltipId} role="tooltip">
        {help}
      </span>
    </>
  );
}
