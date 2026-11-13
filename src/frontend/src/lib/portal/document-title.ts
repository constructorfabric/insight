import { screenLabel } from "@/lib/portal/screen-label";

export const DOCUMENT_TITLE_APP = "Insight";

const CRUMB = " › ";
const DOT = " · ";

export function documentTitle(
  screenPath: string,
  subject?: string | null
): string {
  const named = screenLabel(screenPath);
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

/** The screen a path names, for the title. */
export function titleScreenPath(pathname: string): string {
  const path = strip(pathname);
  // INVARIANT: Home's default view titles as Home, which is `screenLabel("/")`.
  return path === "/home" ? "/" : path;
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

