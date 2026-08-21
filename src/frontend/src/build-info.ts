interface BackendVersions {
  analytics: string;
  identity: string;
}

declare global {
  interface Window {
    __INSIGHT_BUILD__?: {
      frontend: string;
      backend: () => Promise<BackendVersions>;
    };
  }
}

const TIMEOUT_MS = 3000;
const UNREACHABLE = "unreachable";

export function publishBuildInfo(): void {
  window.__INSIGHT_BUILD__ = {
    frontend: import.meta.env.VITE_APP_RELEASE || "unknown",
    backend: readBackendVersions,
  };
}

async function readBackendVersions(): Promise<BackendVersions> {
  const [analytics, identity] = await Promise.all([
    readVersion("/api/analytics/version"),
    readVersion("/api/identity/version"),
  ]);

  return { analytics, identity };
}

async function readVersion(path: string): Promise<string> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), TIMEOUT_MS);

  try {
    const response = await fetch(path, { signal: abort.signal });
    if (!response.ok) throw new Error(`${path} answered ${response.status}`);

    const body = (await response.json()) as { version?: string };
    if (!body.version) throw new Error(`${path} reported no version`);

    return body.version;
  } catch {
    return UNREACHABLE;
  } finally {
    clearTimeout(timer);
  }
}
