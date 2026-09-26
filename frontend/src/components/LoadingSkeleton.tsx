export function LoadingSkeleton({
  label,
  rows = 3,
  inline = false,
}: {
  label: string;
  rows?: number;
  inline?: boolean;
}) {
  return (
    <div
      className={`skeleton-panel${inline ? " skeleton-panel-inline" : " card"}`}
      role="status"
      aria-label={label}
    >
      <span className="sr-only">{label}</span>
      {Array.from({ length: rows }, (_, index) => (
        <div className="skeleton-row" aria-hidden="true" key={index}>
          <span className="skeleton-line skeleton-line-short" />
          <span className="skeleton-line" />
        </div>
      ))}
    </div>
  );
}
