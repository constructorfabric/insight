import { screenLabel } from "@/lib/portal/screen-label";
import { screenOf } from "@/usage-collection";

export const DOCUMENT_TITLE_APP = "Insight";

const CRUMB = " › ";
const DOT = " · ";

// INVARIANT: `screenLabel("/")` is Home. A bare portal and the default
// overview item are that screen; the zone's own label is Overview.
const HOME_PATHS = new Set([
  "/portal",
  "/portal/overview",
  "/portal/overview/at-a-glance",
]);

export function documentTitle(
  screenPath: string,
  subject?: string | null
): string {
  const path = HOME_PATHS.has(strip(screenPath)) ? "/" : screenPath;
  const named = screenLabel(path);
  if (named.startsWith("/")) {
    return finish(subject?.trim() ? [subject.trim()] : []);
  }

  const echoed = pathSegments(screenPath);
  const parts: string[] = [];
  const specific = subject?.trim();
  if (specific) parts.push(specific);
  const crumbs = named.split(CRUMB);
  for (let i = crumbs.length - 1; i >= 0; i -= 1) {
    const crumb = crumbs[i]?.trim();
    if (crumb && !echoed.has(crumb)) parts.push(crumb);
  }
  return finish(parts);
}

export function titleScreenPath(pathname: string, search: string): string {
  const path = strip(pathname);
  if (path.startsWith("/portal/custom")) return path;
  if (path.startsWith("/ic/")) {
    return screenOf({ pathname: path, search: searchWithoutZone(search) });
  }
  return screenOf({ pathname: path, search });
}

export function dashboardSubject(
  title: string | null | undefined,
  name: string | null | undefined
): string | null {
  return title?.trim() || name?.trim() || null;
}

function finish(parts: readonly string[]): string {
  if (parts.length === 0) return DOCUMENT_TITLE_APP;
  return `${parts.join(DOT)}${DOT}${DOCUMENT_TITLE_APP}`;
}

function strip(pathname: string): string {
  if (pathname.length > 1 && pathname.endsWith("/")) {
    return pathname.replace(/\/+$/, "");
  }
  return pathname;
}

function pathSegments(screenPath: string): Set<string> {
  const segments = new Set<string>();
  for (const segment of strip(screenPath).split("/")) {
    if (!segment) continue;
    try {
      segments.add(decodeURIComponent(segment));
    } catch {
      segments.add(segment);
    }
  }
  return segments;
}

function searchWithoutZone(search: string): string {
  const params = new URLSearchParams(search);
  params.delete("zone");
  params.delete("dir");
  params.delete("lens");
  params.delete("mode");
  const rest = params.toString();
  return rest ? `?${rest}` : "";
}
