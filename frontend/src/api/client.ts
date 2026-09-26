export class ApiFailure extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiFailure";
  }
}

function errorField(
  payload: unknown,
  field: "code" | "detail",
  fallback: string,
): string {
  if (typeof payload !== "object" || payload === null) return fallback;
  const value = Reflect.get(payload, field);
  return typeof value === "string" ? value : fallback;
}

function getCookie(name: string): string | undefined {
  return document.cookie
    .split(";")
    .map((value) => value.trim())
    .find((value) => value.startsWith(name + "="))
    ?.slice(name.length + 1);
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const method = init.method ?? "GET";
  const headers = new Headers(init.headers);
  if (method !== "GET" && method !== "HEAD") {
    headers.set("content-type", "application/json");
    const token = getCookie("statusdeck_csrf");
    if (token) headers.set("x-csrf-token", token);
  }
  const response = await fetch(path, {
    ...init,
    headers,
    credentials: "include",
  });
  if (response.status === 204) return undefined as T;
  let payload: unknown;
  try {
    payload = await response.json();
  } catch {
    if (response.ok) {
      throw new ApiFailure(
        response.status,
        "invalid_response",
        "The server returned an invalid response.",
      );
    }
    payload = {};
  }
  if (
    response.status === 401 &&
    errorField(payload, "code", "") !== "invalid_credentials"
  ) {
    window.dispatchEvent(new Event("statusdeck:unauthorized"));
  }
  if (!response.ok)
    throw new ApiFailure(
      response.status,
      errorField(payload, "code", "request_failed"),
      errorField(payload, "detail", "Request failed"),
    );
  return payload as T;
}

export async function ensureCsrf(): Promise<void> {
  await api<void>("/api/v1/csrf");
}
