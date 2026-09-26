import { useId } from "react";
import { useSearchParams } from "react-router-dom";

export function PersonalIncidentFilters({
  heading,
  placeholder,
  providers,
}: {
  heading: string;
  placeholder: string;
  providers: Array<{ id: string; name: string }>;
}) {
  const [searchParams, setSearchParams] = useSearchParams();
  const headingId = useId();
  const filters = searchParams.toString();
  return (
    <form
      className="card incident-filters"
      key={filters}
      onSubmit={(event) => {
        event.preventDefault();
        const form = new FormData(event.currentTarget);
        const next = new URLSearchParams();
        for (const name of ["q", "provider_id"]) {
          const value = form.get(name)?.toString().trim();
          if (value) next.set(name, value);
        }
        setSearchParams(next);
      }}
    >
      <div className="incident-filter-header">
        <div>
          <h2>{heading}</h2>
        </div>
        <div className="filter-actions">
          <button className="button primary" type="submit">
            Apply filters
          </button>
          <button
            className="button ghost"
            type="button"
            onClick={(event) => {
              event.currentTarget.form?.reset();
              setSearchParams({});
            }}
          >
            Clear
          </button>
        </div>
      </div>
      <div
        className="incident-filter-section"
        role="group"
        aria-labelledby={headingId}
      >
        <h3 id={headingId}>Provider and search</h3>
        <div className="filter-grid">
          <label>
            Search
            <input
              name="q"
              maxLength={500}
              defaultValue={searchParams.get("q") ?? ""}
              placeholder={placeholder}
            />
          </label>
          <label>
            Provider
            <select
              name="provider_id"
              defaultValue={searchParams.get("provider_id") ?? ""}
            >
              <option value="">All providers</option>
              {searchParams.has("provider_id") &&
                !providers.some(
                  (p) => p.id === searchParams.get("provider_id"),
                ) && (
                  <option value={searchParams.get("provider_id") ?? ""}>
                    Selected provider
                  </option>
                )}
              {providers.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.name}
                </option>
              ))}
            </select>
          </label>
        </div>
      </div>
    </form>
  );
}
