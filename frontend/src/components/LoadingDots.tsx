export function LoadingDots({ label }: { label: string }) {
  return (
    <span className="loading-dots" role="status">
      <span className="sr-only">{label}</span>
      <span className="loading-dot" aria-hidden="true" />
      <span className="loading-dot" aria-hidden="true" />
      <span className="loading-dot" aria-hidden="true" />
    </span>
  );
}
