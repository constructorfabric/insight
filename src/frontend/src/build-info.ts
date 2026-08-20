declare global {
  interface Window {
    __INSIGHT_BUILD__?: {
      frontend: string;
      backend: () => Promise<Record<string, string>>;
    };
  }
}

const TIMEOUT_MS = 3000;

const BACKENDS: Record<string, string> = {
  analytics: "/api/analytics/version",
  identity: "/api/identity/version",
};

export function publishBuildInfo(): void {
  window.__INSIGHT_BUILD__ = {
    frontend: import.meta.env.VITE_APP_RELEASE || "unknown",
    backend: readBackendVersions,
  };
}

async function readBackendVersions(): Promise<Record<string, string>> {
  const reported = await Promise.all(
    Object.entries(BACKENDS).map(
      async ([name, path]) => [name, await readVersion(path)] as const,
    ),
  );

  return Object.fromEntries(reported);
}

async function readVersion(path: string): Promise<string> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), TIMEOUT_MS);

  try {
    const response = await fetch(path, { signal: abort.signal });
    if (!response.ok) return "unreachable";

    const body = (await response.json()) as { version?: string };
    return body.version ?? "unreachable";
  } catch {
    return "unreachable";
  } finally {
    clearTimeout(timer);
  }
}
