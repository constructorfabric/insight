import { defaultZoneItem } from "@/lib/portal/nav-model";
import { screenLabel } from "@/lib/portal/screen-label";
import { screenOf } from "@/usage-collection";

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

export function titleScreenPath(input: {
  pathname: string;
  search: string;
  zone: string;
  item: string | null;
}): string {
  const path = strip(input.pathname);
  if (path.startsWith("/portal/custom")) return path;

  if (input.zone === "person") {
    const id = path.match(/^\/ic\/([^/]+)/)?.[1] ?? "viewer";
    const base = `/ic/${id}/personal`;
    return input.item ? `${base}/${input.item}` : base;
  }

  if (input.zone === "people") {
    const id = path.match(/^\/ic\/([^/]+)/)?.[1] ?? "viewer";
    const base = `/ic/${id}/team`;
    return input.item ? `${base}/${input.item}` : base;
  }

  // INVARIANT: the overview page titles its default item Home, and that is
  // `screenLabel("/")`. Any other resolved zone, including Person when an
  // install hides Home, keeps its own screen name.
  if (input.zone === "overview" && input.item === defaultZoneItem("overview")) {
    return "/";
  }

  const params = new URLSearchParams(input.search);
  params.set("zone", input.zone);
  if (input.item) params.set("item", input.item);
  else params.delete("item");
  const rest = params.toString();
  return screenOf({ pathname: "/portal", search: rest ? `?${rest}` : "" });
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

