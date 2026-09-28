import { useId, type ReactNode } from "react";

export function FieldHelp({
  label,
  help,
  className = "",
}: {
  label: string;
  help: ReactNode;
  className?: string;
}) {
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
      <span
        className={`field-help-tooltip ${className}`}
        id={tooltipId}
        role="tooltip"
      >
        {help}
      </span>
    </>
  );
}
