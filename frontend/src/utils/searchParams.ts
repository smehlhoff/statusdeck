export function selectSearchParams(
  source: URLSearchParams,
  allowedNames: readonly string[],
): URLSearchParams {
  const selected = new URLSearchParams();
  for (const [name, value] of source) {
    if (allowedNames.includes(name)) selected.append(name, value);
  }
  return selected;
}
