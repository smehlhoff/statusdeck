import type { ReactNode } from "react";

const icons = {
  pulse: <path d="M2 12h6l3-9 4 18 3-9h4" />,
  providers: (
    <path d="M7 19a5 5 0 1 1 .9-9.92A7 7 0 0 1 21 12a3.5 3.5 0 0 1-.5 7Z" />
  ),
  warning: (
    <>
      <path d="M10.27 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.73 3.86a2 2 0 0 0-3.46 0Z" />
      <path d="M12 9v4M12 17h.01" />
    </>
  ),
  bell: <path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4" />,
  chart: <path d="M4 20v-6M9 20V9M14 20V4M19 20V7" />,
  settings: (
    <g transform="translate(1 0)">
      <path d="m9 3-.5 2.2-1.7 1-2.2-.6-2 3.4 1.7 1.6v2.8L2.6 15l2 3.4 2.2-.6 1.7 1L9 21h4l.5-2.2 1.7-1 2.2.6 2-3.4-1.7-1.6v-2.8L19.4 9l-2-3.4-2.2.6-1.7-1L13 3Z" />
      <circle cx="11" cy="12" r="3" />
    </g>
  ),
} satisfies Record<string, ReactNode>;

export function NavigationIcon({ name }: { name: keyof typeof icons }) {
  return (
    <svg
      className="navigation-icon"
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {icons[name]}
    </svg>
  );
}
