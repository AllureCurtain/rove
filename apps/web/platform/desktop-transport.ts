export interface DesktopTransport {
  apiPrefix: string;
  token: string;
}

declare global {
  interface Window {
    __ROVE_API_URL__?: string;
    __ROVE_TOKEN__?: string;
  }
}

export function desktopTransport(): DesktopTransport | null {
  if (typeof window === "undefined") {
    return null;
  }
  const rawBase = window.__ROVE_API_URL__?.trim();
  const token = window.__ROVE_TOKEN__?.trim();
  if (!rawBase || !token) {
    return null;
  }
  let base: URL;
  try {
    base = new URL(rawBase);
  } catch {
    return null;
  }
  if (
    base.protocol !== "http:" ||
    !["127.0.0.1", "localhost", "[::1]"].includes(base.hostname) ||
    base.username !== "" ||
    base.password !== "" ||
    base.search !== "" ||
    base.hash !== ""
  ) {
    return null;
  }
  base.pathname = base.pathname.replace(/\/+$/, "");
  return { apiPrefix: base.toString().replace(/\/$/, ""), token };
}

export function withDesktopAuthorization(
  fetchImpl: typeof globalThis.fetch,
  token: string | undefined,
): typeof globalThis.fetch {
  const bearer = token?.trim();
  if (!bearer) {
    return fetchImpl;
  }
  return ((input: RequestInfo | URL, init?: RequestInit) => {
    const headers = new Headers(init?.headers);
    headers.set("authorization", `Bearer ${bearer}`);
    return fetchImpl(input, { ...init, headers });
  }) as typeof globalThis.fetch;
}
