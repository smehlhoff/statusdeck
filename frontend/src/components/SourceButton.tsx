interface SourceButtonProps {
  href: string;
  className?: string;
}

export function SourceButton({ href, className = "" }: SourceButtonProps) {
  return (
    <a
      className={`button ghost${className ? ` ${className}` : ""}`}
      href={href}
      target="_blank"
      rel="noreferrer"
    >
      Source
    </a>
  );
}
