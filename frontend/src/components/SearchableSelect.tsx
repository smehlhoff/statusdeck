import { useEffect, useId, useRef, useState } from "react";

export function SearchableSelect({
  value,
  onChange,
  options,
  label,
  searchLabel,
  placeholder,
  emptyMessage,
  describedBy,
  hint,
  hideLabel = false,
}: {
  value: string;
  onChange: (value: string) => void;
  options: Array<{ value: string; label: string }>;
  label: string;
  searchLabel: string;
  placeholder: string;
  emptyMessage: string;
  describedBy?: string;
  hint?: string;
  hideLabel?: boolean;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const terms = search
    .toLocaleLowerCase()
    .replaceAll("_", " ")
    .trim()
    .split(/\s+/);
  const results = options.filter((option) =>
    terms.every((term) =>
      option.label.toLocaleLowerCase().replaceAll("_", " ").includes(term),
    ),
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

  function choose(value: string) {
    onChange(value);
    setOpen(false);
    buttonRef.current?.focus();
  }

  return (
    <div
      className="searchable-select"
      ref={rootRef}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <label
        id={`${id}-label`}
        htmlFor={`${id}-trigger`}
        className={hideLabel ? "sr-only" : undefined}
      >
        {label}
      </label>
      <button
        id={`${id}-trigger`}
        ref={buttonRef}
        type="button"
        className="searchable-select-trigger"
        aria-labelledby={`${id}-label ${id}-value`}
        aria-describedby={describedBy}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? `${id}-list` : undefined}
        onClick={() => {
          setSearch("");
          setActiveIndex(0);
          setOpen((current) => !current);
        }}
      >
        <span id={`${id}-value`}>
          {options.find((option) => option.value === value)?.label ?? value}
        </span>
        <span aria-hidden="true">▾</span>
      </button>
      {open && (
        <div className="searchable-select-popup">
          {hint && (
            <p className="muted" role="status">
              {hint}
            </p>
          )}
          <input
            ref={inputRef}
            role="combobox"
            aria-label={searchLabel}
            aria-expanded="true"
            aria-controls={`${id}-list`}
            aria-autocomplete="list"
            aria-activedescendant={
              results[activeIndex] ? `${id}-option-${activeIndex}` : undefined
            }
            placeholder={placeholder}
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
                const option = results[activeIndex];
                if (option) choose(option.value);
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
            aria-label={label}
            className="searchable-select-options"
          >
            {results.map((option, index) => (
              <div
                role="option"
                id={`${id}-option-${index}`}
                key={option.value}
                aria-selected={option.value === value}
                className={
                  index === activeIndex
                    ? "searchable-select-option active"
                    : "searchable-select-option"
                }
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => choose(option.value)}
              >
                {option.label}
                {option.value === value && <span aria-hidden="true"> ✓</span>}
              </div>
            ))}
          </div>
          {results.length === 0 && (
            <p className="muted" role="status">
              {emptyMessage}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
