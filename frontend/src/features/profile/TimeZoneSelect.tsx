import { useEffect, useId, useRef, useState } from "react";

const LOCAL_ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;
const CAN_LIST_TIME_ZONES = typeof Intl.supportedValuesOf === "function";
const TIME_ZONES = Array.from(
  new Set([
    "UTC",
    LOCAL_ZONE,
    ...(CAN_LIST_TIME_ZONES ? Intl.supportedValuesOf("timeZone") : []),
  ]),
).sort();

function zoneLabel(zone: string) {
  return zone === "browser"
    ? `Local time (${LOCAL_ZONE.replaceAll("_", " ")})`
    : zone.replaceAll("_", " ");
}

export function TimeZoneSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (zone: string) => void;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const zones = ["browser", ...TIME_ZONES];
  if (!zones.includes(value)) zones.push(value);
  const terms = search
    .toLocaleLowerCase()
    .replaceAll("_", " ")
    .trim()
    .split(/\s+/);
  const results = zones.filter((zone) =>
    terms.every((term) => zoneLabel(zone).toLocaleLowerCase().includes(term)),
  );

  useEffect(() => {
    if (!open) return;
    inputRef.current?.focus();

    function outside(event: PointerEvent) {
      if (
        event.target instanceof Node &&
        !rootRef.current?.contains(event.target)
      )
        setOpen(false);
    }

    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  useEffect(() => {
    if (open)
      document
        .getElementById(`${id}-option-${activeIndex}`)
        ?.scrollIntoView({ block: "nearest" });
  }, [open, activeIndex, search, id]);

  function choose(zone: string) {
    onChange(zone);
    setOpen(false);
    buttonRef.current?.focus();
  }

  return (
    <div
      className="timezone-select"
      ref={rootRef}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <label id={`${id}-label`} htmlFor={`${id}-trigger`}>
        Time zone
      </label>
      <button
        id={`${id}-trigger`}
        ref={buttonRef}
        type="button"
        className="timezone-trigger"
        aria-labelledby={`${id}-label ${id}-value`}
        aria-describedby="profile-time-zone-help"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? `${id}-list` : undefined}
        onClick={() => {
          setSearch("");
          setActiveIndex(0);
          setOpen((current) => !current);
        }}
      >
        <span id={`${id}-value`}>{zoneLabel(value)}</span>
        <span aria-hidden="true">▾</span>
      </button>
      {open && (
        <div className="timezone-popup">
          {!CAN_LIST_TIME_ZONES && (
            <p className="muted" role="status">
              This browser can list only UTC, local time, and your saved time
              zone.
            </p>
          )}
          <input
            ref={inputRef}
            role="combobox"
            aria-label="Search time zones"
            aria-expanded="true"
            aria-controls={`${id}-list`}
            aria-autocomplete="list"
            aria-activedescendant={
              results[activeIndex] ? `${id}-option-${activeIndex}` : undefined
            }
            placeholder="Search city or time zone…"
            autoComplete="off"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              setActiveIndex(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                const direction = event.key === "ArrowDown" ? 1 : -1;
                setActiveIndex((index) =>
                  Math.max(0, Math.min(results.length - 1, index + direction)),
                );
              } else if (event.key === "Enter") {
                event.preventDefault();
                const zone = results[activeIndex];
                if (zone) choose(zone);
              } else if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                setOpen(false);
                buttonRef.current?.focus();
              }
            }}
          />
          <div
            role="listbox"
            id={`${id}-list`}
            aria-label="Time zones"
            className="timezone-options"
          >
            {results.map((zone, index) => (
              <div
                role="option"
                id={`${id}-option-${index}`}
                key={zone}
                aria-selected={zone === value}
                className={
                  index === activeIndex
                    ? "timezone-option active"
                    : "timezone-option"
                }
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => choose(zone)}
              >
                {zoneLabel(zone)}
                {zone === value && <span aria-hidden="true"> ✓</span>}
              </div>
            ))}
          </div>
          {results.length === 0 && (
            <p className="muted" role="status">
              No matching time zones.
            </p>
          )}
        </div>
      )}
    </div>
  );
}
