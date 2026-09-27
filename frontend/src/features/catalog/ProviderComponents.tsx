import { useState } from "react";
import type { Component, Status } from "../../api/types";
import { StatusBadge } from "../../components/StatusBadge";
import { humanizeIdentifier } from "../../utils/display";
import "./provider-components.css";

const STATUS_ORDER: Record<Status, number> = {
  operational: 0,
  maintenance: 1,
  unknown: 2,
  degraded: 3,
  partial_outage: 4,
  major_outage: 5,
};

function statusRank(component: Component) {
  return component.active ? STATUS_ORDER[component.status ?? "unknown"] : -1;
}

function statusText(component: Component) {
  if (!component.active) return "No longer reported by provider";
  return component.status
    ? humanizeIdentifier(component.status)
    : "Not checked";
}

function ComponentRow({
  component,
  nested = false,
}: {
  component: Component;
  nested?: boolean;
}) {
  return (
    <div className={`pc-row${nested ? " pc-child" : ""}`}>
      <div>
        <span>{component.name}</span>
        {component.description && (
          <small className="pc-description">{component.description}</small>
        )}
      </div>
      <span>{component.group ?? "—"}</span>
      <span className="pc-status">
        <StatusBadge
          value={component.active ? (component.status ?? "unknown") : "neutral"}
          label={statusText(component)}
        />
      </span>
    </div>
  );
}

export function ProviderComponents({
  components,
}: {
  components: Component[];
}) {
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [group, setGroup] = useState("");
  const query = search.trim().toLowerCase();
  const groups = [
    ...new Set(
      components.flatMap((component) =>
        component.group ? [component.group] : [],
      ),
    ),
  ].sort((a, b) => a.localeCompare(b));
  const visible = components.filter(
    (component) =>
      `${component.name} ${component.group ?? ""} ${component.upstream_id}`
        .toLowerCase()
        .includes(query) &&
      (!group || component.group === group) &&
      (filter !== "selected" || component.selected) &&
      (filter !== "affected" || statusRank(component) >= STATUS_ORDER.degraded),
  );
  const services = new Map<string, Component[]>();
  for (const component of visible) {
    const entries = services.get(component.name);
    if (entries) entries.push(component);
    else services.set(component.name, [component]);
  }
  const rows = [...services]
    .map(([name, entries]) => ({
      name,
      entries: entries.sort((a, b) =>
        (a.group ?? "").localeCompare(b.group ?? ""),
      ),
      worst: entries.reduce((worst, item) =>
        statusRank(item) > statusRank(worst) ? item : worst,
      ),
      affected: entries.filter(
        (item) => statusRank(item) >= STATUS_ORDER.degraded,
      ).length,
    }))
    .sort((a, b) => a.name.localeCompare(b.name));

  return (
    <section className="card provider-detail-section provider-components-section">
      <div className="section-heading">
        <h2>Components</h2>
        <span className="muted" role="status">
          {visible.length.toLocaleString()} of{" "}
          {components.length.toLocaleString()} components
        </span>
      </div>
      <div className="catalog-toolbar pc-toolbar">
        <label className="search">
          <span className="sr-only">Search components</span>
          <input
            type="search"
            placeholder="Search services or regions"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
        <div className="pc-filters" role="group" aria-label="Component filters">
          {[
            ["all", "All"],
            ["selected", "Monitored"],
            ["affected", "Affected"],
          ].map(([value, label]) => (
            <button
              key={value}
              type="button"
              className={`button ${filter === value ? "primary" : "ghost"}`}
              aria-pressed={filter === value}
              onClick={() => setFilter(value)}
            >
              {label}
            </button>
          ))}
        </div>
        {groups.length > 0 && (
          <label className="pc-region">
            <span className="sr-only">Component region or group</span>
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
      <div className="pc-row pc-head" aria-hidden="true">
        <span>Service</span>
        <span>Region</span>
        <span>Status</span>
      </div>
      <div
        className="pc-list"
        role="region"
        aria-label="Component list"
        tabIndex={0}
      >
        {rows.map(({ name, entries, worst, affected }) =>
          entries.length === 1 ? (
            <ComponentRow key={name} component={entries[0]} />
          ) : (
            <details
              key={name}
              className="pc-service"
              open={Boolean(query || group || filter !== "all")}
            >
              <summary className="pc-row">
                <strong className="pc-service-name">{name}</strong>
                <span>{entries.length} components</span>
                <span className="pc-status">
                  <StatusBadge
                    value={
                      worst.active ? (worst.status ?? "unknown") : "neutral"
                    }
                    label={
                      affected > 0
                        ? `${affected} ${affected === 1 ? "component" : "components"} affected`
                        : statusText(worst)
                    }
                  />
                </span>
              </summary>
              {entries.map((component) => (
                <ComponentRow key={component.id} component={component} nested />
              ))}
            </details>
          ),
        )}
      </div>
      {visible.length === 0 && (
        <p className="muted" role="status">
          No components match these filters.
        </p>
      )}
    </section>
  );
}
