import type { ReactNode } from "react";

export function EmptyState({
  title,
  description,
  action,
  inline = false,
  error = false,
}: {
  title: string;
  description: string;
  action?: ReactNode;
  inline?: boolean;
  error?: boolean;
}) {
  return (
    <div
      className={`empty-state${inline ? " empty-state-inline" : " card"}${error ? " error-state" : ""}`}
      role={error ? "alert" : undefined}
    >
      <h2>{title}</h2>
      <p className="muted">{description}</p>
      {action && <div className="empty-state-action">{action}</div>}
    </div>
  );
}
