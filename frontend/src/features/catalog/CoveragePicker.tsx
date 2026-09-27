import { useRef, useState } from "react";
import type { Component } from "../../api/types";

function SelectionCheckbox({
  label,
  count,
  total,
  disabled,
  onChange,
}: {
  label: string;
  count: number;
  total: number;
  disabled: boolean;
  onChange: (selected: boolean) => void;
}) {
  return (
    <input
      type="checkbox"
      aria-label={label}
      checked={total > 0 && count === total}
      ref={(input) => {
        if (input) input.indeterminate = count > 0 && count < total;
      }}
      disabled={disabled}
      onClick={(event) => event.stopPropagation()}
      onChange={(event) => onChange(event.target.checked)}
    />
  );
}

export function CoveragePicker({
  components,
  selectedIds,
  hidden,
  disabled,
  onSelect,
}: {
  components: Component[];
  selectedIds: Set<string>;
  hidden: boolean;
  disabled: boolean;
  onSelect: (ids: string[], selected: boolean) => void;
}) {
  const [initiallySelectedServices] = useState(
    () =>
      new Set(
        components
          .filter((item) => selectedIds.has(item.id))
          .map((item) => item.name),
      ),
  );
  const [search, setSearch] = useState("");
  const [group, setGroup] = useState("");
  const [selectedOnly, setSelectedOnly] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const query = search.trim().toLowerCase();
  const groups = [
    ...new Set(components.flatMap((item) => (item.group ? [item.group] : []))),
  ].sort((a, b) => a.localeCompare(b));
  const visible = components.filter(
    (item) =>
      `${item.name} ${item.group ?? ""} ${item.upstream_id}`
        .toLowerCase()
        .includes(query) &&
      (!group || item.group === group) &&
      (!selectedOnly || selectedIds.has(item.id)),
  );
  const services = new Map<string, Component[]>();
  for (const item of visible) {
    const entries = services.get(item.name);
    if (entries) entries.push(item);
    else services.set(item.name, [item]);
  }
  const matchingIds = visible.map((item) => item.id);
  const matchingSelected = visible.filter((item) =>
    selectedIds.has(item.id),
  ).length;

  return (
    <div className="coverage-picker" hidden={hidden}>
      <div className="coverage-search">
        <label>
          <span className="sr-only">Search components</span>
          <input
            ref={searchRef}
            type="search"
            placeholder="Search components"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
        {groups.length > 0 && (
          <label>
            <span className="sr-only">Region or group</span>
            <select
              value={group}
              onChange={(event) => setGroup(event.target.value)}
            >
              <option value="">All regions / groups</option>
              {groups.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>
      <div className="coverage-filter-bar">
        <label className="check">
          <input
            type="checkbox"
            checked={selectedOnly}
            onChange={(event) => setSelectedOnly(event.target.checked)}
          />
          Selected only
        </label>
        <div className="coverage-bulk">
          <button
            type="button"
            className="button compact ghost"
            disabled={
              disabled ||
              visible.length === 0 ||
              matchingSelected === visible.length
            }
            onClick={() => onSelect(matchingIds, true)}
          >
            Select matches
          </button>
          <button
            type="button"
            className="button compact ghost"
            disabled={disabled || matchingSelected === 0}
            onClick={() => onSelect(matchingIds, false)}
          >
            Clear matches
          </button>
        </div>
        <span className="muted coverage-result-count" role="status">
          {(query || group) &&
            `${visible.length.toLocaleString()} result${visible.length === 1 ? "" : "s"}`}
        </span>
      </div>
      <div
        className="coverage-service-list"
        role="region"
        aria-label="Available components"
        tabIndex={0}
      >
        {[...services]
          .sort(
            ([a], [b]) =>
              Number(initiallySelectedServices.has(b)) -
                Number(initiallySelectedServices.has(a)) || a.localeCompare(b),
          )
          .map(([name, entries]) => {
            const selectedCount = entries.filter((item) =>
              selectedIds.has(item.id),
            ).length;
            return (
              <details
                key={name}
                className="coverage-service"
                open={Boolean(query || group || selectedOnly) || undefined}
              >
                <summary>
                  <SelectionCheckbox
                    label={`Select matching components for ${name}`}
                    count={selectedCount}
                    total={entries.length}
                    disabled={disabled}
                    onChange={(selected) =>
                      onSelect(
                        entries.map((item) => item.id),
                        selected,
                      )
                    }
                  />
                  <strong>{name}</strong>
                  <span
                    className="muted"
                    title={`${selectedCount} of ${entries.length} selected`}
                  >
                    {selectedCount} / {entries.length}
                  </span>
                </summary>
                {entries
                  .sort((a, b) => (a.group ?? "").localeCompare(b.group ?? ""))
                  .map((item) => (
                    <label className="check coverage-component" key={item.id}>
                      <input
                        type="checkbox"
                        checked={selectedIds.has(item.id)}
                        disabled={disabled}
                        onChange={(event) =>
                          onSelect([item.id], event.target.checked)
                        }
                      />
                      <span>
                        <span>{item.group ?? item.name}</span>
                        {!item.active && (
                          <small className="coverage-inactive">
                            No longer reported by provider
                          </small>
                        )}
                      </span>
                    </label>
                  ))}
              </details>
            );
          })}
        {visible.length === 0 && (
          <div className="coverage-empty" role="status">
            <p>No components match these filters.</p>
            <button
              className="button ghost"
              type="button"
              onClick={() => {
                setSearch("");
                setGroup("");
                setSelectedOnly(false);
                searchRef.current?.focus();
              }}
            >
              Clear filters
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
