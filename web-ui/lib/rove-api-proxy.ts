const DEFAULT_API_BASE = "http://127.0.0.1:8787";

const HOP_BY_HOP_HEADERS = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
]);

export interface RoveApiProxyOptions {
  apiBase?: string;
  apiToken?: string;
  fetchImpl?: typeof fetch;
}

export async function proxyRoveApiRequest(
  request: Request,
  pathSegments: string[],
  options: RoveApiProxyOptions = {},
): Promise<Response> {
  const fetchImpl = options.fetchImpl ?? fetch;
  const upstreamUrl = buildUpstreamUrl(
    request.url,
    pathSegments,
    options.apiBase ?? process.env.ROVE_API_BASE ?? DEFAULT_API_BASE,
  );
  const headers = proxyRequestHeaders(
    request.headers,
    options.apiToken ?? process.env.ROVE_API_TOKEN,
  );
  const upstream = await fetchImpl(upstreamUrl, {
    method: request.method,
    headers,
    body: request.body,
    redirect: "manual",
    duplex: "half",
  } as RequestInit);

  return new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: proxyResponseHeaders(upstream.headers),
  });
}

function buildUpstreamUrl(
  requestUrl: string,
  pathSegments: string[],
  apiBase: string,
): string {
  const request = new URL(requestUrl);
  const base = new URL(apiBase.endsWith("/") ? apiBase : `${apiBase}/`);
  const encodedPath = pathSegments.map(encodeURIComponent).join("/");
  base.pathname = joinPath(base.pathname, encodedPath);
  base.search = request.search;
  return base.toString();
}

function joinPath(basePath: string, path: string): string {
  const trimmedBase = basePath.replace(/\/+$/, "");
  const trimmedPath = path.replace(/^\/+/, "");
  if (!trimmedPath) {
    return trimmedBase || "/";
  }
  return `${trimmedBase}/${trimmedPath}`;
}

function proxyRequestHeaders(
  source: Headers,
  apiToken: string | undefined,
): Headers {
  const headers = new Headers();
  source.forEach((value, key) => {
    if (!HOP_BY_HOP_HEADERS.has(key) && key !== "host" && key !== "content-length") {
      headers.set(key, value);
    }
  });
  const token = apiToken?.trim();
  if (token) {
    headers.set("authorization", `Bearer ${token}`);
  } else {
    headers.delete("authorization");
  }
  return headers;
}

function proxyResponseHeaders(source: Headers): Headers {
  const headers = new Headers();
  source.forEach((value, key) => {
    if (!HOP_BY_HOP_HEADERS.has(key) && key !== "content-length") {
      headers.set(key, value);
    }
  });
  return headers;
}
