export function UserAvatar({
  name,
  large = false,
}: {
  name: string;
  large?: boolean;
}) {
  const words = name.trim().split(/\s+/u);
  const first = Array.from(words[0] ?? "");
  const last = Array.from(words.at(-1) ?? "");
  const initials =
    (words.length > 1
      ? `${first[0] ?? ""}${last[0] ?? ""}`
      : first.slice(0, 2).join("")) || "AD";
  return (
    <span
      className={`user-avatar${large ? " user-avatar-large" : ""}`}
      aria-hidden="true"
    >
      {initials.toLocaleUpperCase()}
    </span>
  );
}
