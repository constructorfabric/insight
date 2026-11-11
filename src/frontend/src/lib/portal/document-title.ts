import type { EditableKind } from "@/api/custom-client";
import i18n from "@/i18n";
import { resolveText } from "@/i18n/localized-text";
import { NOUNS } from "@/lib/custom/editor/kinds";
import { GROUPS } from "@/lib/insight/groups";
import { MODE_LABELS, resolveMode } from "@/lib/portal/identity-modes";
import {
  DIRECTIONS,
  lensBySlug,
  peopleItemsFor,
  resolveZoneItem,
  zoneById,
  zoneItems,
  type Direction,
} from "@/lib/portal/nav-model";

export const DOCUMENT_TITLE_APP = "Insight";

const SEPARATOR = " · ";

// INVARIANT: the zone id is `overview`; the rail and the home heading say Home.
const HOME = "Home";

export interface PageTitleInput {
  pathname: string;
  zone: string | null;
  item: string | null;
  dir: string;
  lens: string;
  mode?: string;
  peopleFlat: boolean;
}

export function formatDocumentTitle(parts: readonly string[]): string {
  const kept: string[] = [];
  for (const part of parts) {
    const trimmed = part.trim();
    if (trimmed) kept.push(trimmed);
  }
  if (kept.length === 0) return DOCUMENT_TITLE_APP;

  return `${kept.join(SEPARATOR)}${SEPARATOR}${DOCUMENT_TITLE_APP}`;
}

export function pageDocumentTitle(input: PageTitleInput): string {
  return formatDocumentTitle(titleParts(input));
}

export function dashboardDisplayName(
  title: string | null | undefined,
  name: string | null | undefined
): string | null {
  const heading = title?.trim() || name?.trim() || "";
  return heading || null;
}

export function dashboardDocumentTitle(
  displayName: string | null
): string | null {
  const name = displayName?.trim() ?? "";
  if (!name) return null;

  return formatDocumentTitle([name, "Dashboards"]);
}

export function compareDocumentTitle(names: readonly string[]): string {
  const kept = names
    .map((name) => name.trim())
    .filter((name) => name.length > 0);
  const lead = kept.length === 0 ? "Compare" : kept.join(", ");

  return formatDocumentTitle([lead, "Metrics view", "People"]);
}

export function personSectionTitle(item: string | null): string | null {
  if (!item) return null;

  return GROUPS.find((group) => group.id === item)?.title ?? null;
}

export function personDocumentTitle(
  name: string,
  section: string | null
): string {
  const page = section?.trim();
  return formatDocumentTitle(page ? [page, name, "Person"] : [name, "Person"]);
}

export function teamDocumentTitle(teamName: string): string | null {
  const name = teamName.trim();
  if (!name) return null;

  return formatDocumentTitle([`${name}'s team`, "People"]);
}

function titleParts(input: PageTitleInput): string[] {
  const custom = customParts(input.pathname);
  if (custom) return custom;

  const zone = zoneFromPath(input);
  if (zone === "person") return personParts(input.item);
  if (zone === "people") {
    return [peopleLabel(input.item, input.peopleFlat), "People"];
  }
  if (zone === "directions") return directionParts(input.dir, input.lens);
  if (zone === "overview" || (zone == null && portalRoot(input.pathname))) {
    return overviewParts(input.item);
  }
  if (zone === "manage") return manageParts(input.item, input.mode);
  if (zone === "aicost") return aiCostParts(input.item);
  if (zone) return namedZoneParts(zone, input.item);

  return [];
}

function zoneFromPath(input: PageTitleInput): string | null {
  const path = stripTrailingSlash(input.pathname);
  if (/^\/portal\/custom(\/|$)/.test(path)) return "custom";
  if (/^\/ic\/[^/]+\/team$/.test(path)) return "people";
  if (/^\/ic\/[^/]+\/personal$/.test(path)) return "person";

  return input.zone;
}

function portalRoot(pathname: string): boolean {
  const path = stripTrailingSlash(pathname);
  return path === "/" || path === "/portal";
}

function personParts(item: string | null): string[] {
  const section = personSectionTitle(item);
  return section ? [section, "Person"] : ["Person"];
}

function peopleLabel(item: string | null, flat: boolean): string {
  const resolved = resolveZoneItem("people", item) ?? "roster";
  const found = peopleItemsFor(flat).find(
    (candidate) => candidate.id === resolved
  );
  return found ? resolveText(i18n.t, found.label) : resolved;
}

function directionParts(dir: string, lens: string): string[] {
  const direction = DIRECTIONS.find((candidate) => candidate.id === dir);
  if (!direction) return ["Directions"];

  const lensName = namedLens(direction, lens);
  if (!lensName) return [direction.name, "Directions"];

  return [lensName, direction.name, "Directions"];
}

function namedLens(direction: Direction, lens: string): string {
  const trimmed = lens.trim();
  if (!trimmed) return "";
  if (direction.lenses.includes(trimmed)) return trimmed;

  return lensBySlug(direction, trimmed) ?? trimmed;
}

function overviewParts(item: string | null): string[] {
  const resolved = resolveZoneItem("overview", item);
  if (!resolved || resolved === "at-a-glance") return [HOME];

  return [paneLabel("overview", resolved), HOME];
}

function aiCostParts(item: string | null): string[] {
  const label = zoneById("aicost")?.label ?? "AI & Cost";
  const resolved = resolveZoneItem("aicost", item);
  if (!resolved || resolved === "overview") return [label];

  return [paneLabel("aicost", resolved), label];
}

function manageParts(item: string | null, mode: string | undefined): string[] {
  const resolved = resolveZoneItem("manage", item);
  if (!resolved) return ["Manage"];

  const label = paneLabel("manage", resolved);
  if (resolved !== "identities" || !mode) return [label, "Manage"];

  const resolvedMode = resolveMode(mode);
  const modeLabel = MODE_LABELS[resolvedMode] ?? resolvedMode;
  return [modeLabel, label, "Manage"];
}

function namedZoneParts(zoneId: string, item: string | null): string[] {
  const zone = zoneById(zoneId);
  if (!zone) return [];

  const resolved = resolveZoneItem(zoneId, item);
  if (!resolved) return [zone.label];

  return [paneLabel(zoneId, resolved), zone.label];
}

function paneLabel(zoneId: string, itemId: string): string {
  const item = zoneItems(zoneId).find((candidate) => candidate.id === itemId);
  return item ? resolveText(i18n.t, item.label) : itemId;
}

function customParts(pathname: string): string[] | null {
  const parts = pathSegments(pathname);
  if (parts[0] !== "portal" || parts[1] !== "custom") return null;

  const [page, second, third] = parts.slice(2);
  if (!page) return ["Dashboards"];
  if (page === "metrics") return ["Metrics", "Dashboards"];
  if (page === "widgets") return ["Widgets", "Dashboards"];
  if (page === "datasets") return ["Datasets", "Dashboards"];
  if (page === "alerts") return alertParts(second, third);
  if (page === "new") return editorParts("New", second);
  if (page === "edit") return editorParts("Edit", second);

  return ["Dashboards"];
}

function alertParts(
  second: string | undefined,
  third: string | undefined
): string[] {
  if (!second) return ["Alerts", "Dashboards"];
  if (second === "new") return ["New alert", "Alerts", "Dashboards"];
  if (third === "edit") return ["Edit alert", "Alerts", "Dashboards"];

  return ["Alert", "Alerts", "Dashboards"];
}

function editorParts(verb: "New" | "Edit", kind: string | undefined): string[] {
  if (!kind || !editableKind(kind)) return ["Dashboards"];

  return [`${verb} ${NOUNS[kind]}`, "Dashboards"];
}

function editableKind(kind: string): kind is EditableKind {
  return Object.hasOwn(NOUNS, kind);
}

function pathSegments(pathname: string): string[] {
  return stripTrailingSlash(pathname)
    .split("/")
    .filter((segment) => segment.length > 0)
    .map(decodeSegment);
}

function stripTrailingSlash(pathname: string): string {
  if (pathname.length > 1 && pathname.endsWith("/")) {
    return pathname.replace(/\/+$/, "");
  }
  return pathname;
}

function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}
