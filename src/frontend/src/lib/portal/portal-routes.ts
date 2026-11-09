import type { ToOptions } from "@tanstack/react-router";

import { DIRECTIONS, lensBySlug, lensSlug } from "@/lib/portal/nav-model";

/**
 * The portal's URL vocabulary: which path a zone, item, direction or person
 * section lives at, and back.
 *
 * Paths use short names; the zone and item ids stay the ones the install's
 * navigation policy (`frontend.nav` in the chart) is written against, so a
 * renamed URL never changes what an install hides.
 */

type SlugTable = readonly (readonly [slug: string, item: string])[];

/** Home's views; the bare `/home` is At a glance. */
export const HOME_VIEWS: SlugTable = [
  ["directions", "by-direction"],
  ["attention", "attention"],
];
const HOME_DEFAULT = "at-a-glance";

/** AI & Cost views; the bare `/explore/ai` is its overview. */
export const AI_VIEWS: SlugTable = [
  ["funnel", "adoption-funnel"],
  ["units", "by-unit-role"],
  ["tools", "per-tool"],
  ["autofix", "autofix"],
  ["audit", "ai-audit"],
  ["spend", "spend-by-tool"],
  ["costs", "cost-by-unit"],
  ["seats", "idle-seats"],
  ["credits", "credits"],
  ["pricing", "ai-pricing"],
];
const AI_DEFAULT = "overview";

export const REPORT_VIEWS: SlugTable = [
  ["builder", "report-builder"],
  ["snapshots", "snapshots"],
  ["export", "export"],
  ["templates", "templates"],
];

export const MANAGE_SURFACES: SlugTable = [
  ["health", "connector-health"],
  ["exclusions", "exclusions"],
  ["snapshots", "snapshots"],
  ["identities", "identities"],
  ["access", "access"],
  ["groups", "group-mgmt"],
  ["ingestion", "ingestion"],
  ["previews", "previews"],
  ["assistant", "ai-assistant"],
  ["news", "whats-new"],
  ["usage", "platform-usage"],
  ["mcp", "mcp"],
  ["config", "config"],
  ["scorecards", "scorecard-mgmt"],
];

/** A person's team views; the bare `/people/$person/team` is the roster. */
export const TEAM_VIEWS: SlugTable = [
  ["roster", "employees"],
  ["metrics", "metrics"],
];
const TEAM_DEFAULT = "roster";

function itemOf(table: SlugTable, slug: string): string | undefined {
  return table.find(([candidate]) => candidate === slug)?.[1];
}

function slugOf(table: SlugTable, item: string): string | undefined {
  return table.find(([, candidate]) => candidate === item)?.[0];
}

export const homeItem = (slug: string) => itemOf(HOME_VIEWS, slug);
export const aiItem = (slug: string) => itemOf(AI_VIEWS, slug);
export const reportItem = (slug: string) => itemOf(REPORT_VIEWS, slug);
export const manageItem = (slug: string) => itemOf(MANAGE_SURFACES, slug);
export const teamItem = (slug: string) => itemOf(TEAM_VIEWS, slug);

/** Person sections are group ids (`git_output`); the URL spells them with dashes. */
export const sectionSlug = (section: string) => section.replaceAll("_", "-");
export const sectionOf = (slug: string) => slug.replaceAll("-", "_");

export function directionLens(
  dir: string,
  slug: string,
): { dir: string; lens: string } | undefined {
  const direction = DIRECTIONS.find((candidate) => candidate.id === dir);
  if (!direction) return undefined;
  const lens = lensBySlug(direction, slug);
  return lens ? { dir, lens } : undefined;
}

export type NavTarget =
  | { zone: "overview"; item?: string }
  | { zone: "directions"; dir?: string; lens?: string }
  | { zone: "aicost"; item?: string }
  | { zone: "person"; person: string; section?: string }
  | { zone: "people"; person: string; item?: string }
  | { zone: "reports"; item?: string }
  | { zone: "manage"; item?: string }
  | { zone: "custom" };

export interface NavLink {
  to: string;
  params: Record<string, string>;
}

function withView(base: string, table: SlugTable, item?: string, bare?: string): NavLink {
  const slug = item && item !== bare ? slugOf(table, item) : undefined;
  return slug
    ? { to: `${base}/$view`, params: { view: slug } }
    : { to: base, params: {} };
}

/**
 * The router options for a navigation target, for `<Link>`, `navigate` and
 * `redirect` alike.
 */
export function linkTo(target: NavTarget): ToOptions {
  // The target is resolved at runtime, so the path is a string the router's
  // literal route types cannot follow; every path built here is a registered route.
  return pathOf(target) as ToOptions;
}

/** The path a navigation target lives at. Unknown items fall back to the zone's default. */
export function pathOf(target: NavTarget): NavLink {
  switch (target.zone) {
    case "overview":
      return withView("/home", HOME_VIEWS, target.item, HOME_DEFAULT);
    case "aicost":
      return withView("/explore/ai", AI_VIEWS, target.item, AI_DEFAULT);
    case "reports":
      return withView("/reports", REPORT_VIEWS, target.item);
    case "manage":
      return manageLink(target.item);
    case "custom":
      return { to: "/dashboards", params: {} };
    case "directions":
      return directionLink(target.dir, target.lens);
    case "person":
      return target.section
        ? {
            to: "/people/$person/$section",
            params: { person: target.person, section: sectionSlug(target.section) },
          }
        : { to: "/people/$person", params: { person: target.person } };
    case "people": {
      const slug =
        target.item && target.item !== TEAM_DEFAULT
          ? slugOf(TEAM_VIEWS, target.item)
          : undefined;
      return slug
        ? { to: "/people/$person/team/$view", params: { person: target.person, view: slug } }
        : { to: "/people/$person/team", params: { person: target.person } };
    }
  }
}

/** Where the reader is, in the zone/item vocabulary the pane and the nav policy speak. */
export interface NavLocation {
  zone: string;
  item: string | null;
  person: string | null;
  dir: string;
  lens: string;
}

/**
 * Read a matched route back into a location. `zone` and a fixed `item` come
 * from the route's static data; everything else from its params.
 */
export function locationOf(
  zone: string,
  fixedItem: string | undefined,
  params: Partial<Record<string, string>>,
): NavLocation {
  const base: NavLocation = { zone, item: fixedItem ?? null, person: null, dir: "", lens: "" };
  const { view, surface, section, person, dir, lens } = params;

  switch (zone) {
    case "overview":
      return { ...base, item: view ? (homeItem(view) ?? null) : HOME_DEFAULT };
    case "aicost":
      return { ...base, item: view ? (aiItem(view) ?? null) : AI_DEFAULT };
    case "reports":
      return { ...base, item: view ? (reportItem(view) ?? null) : base.item };
    case "manage":
      return { ...base, item: surface ? (manageItem(surface) ?? null) : base.item };
    case "directions":
      return {
        ...base,
        dir: dir ?? "",
        lens: (dir && lens && directionLens(dir, lens)?.lens) || "",
      };
    case "person":
      return { ...base, person: person ?? null, item: section ? sectionOf(section) : null };
    case "people":
      return {
        ...base,
        person: person ?? null,
        item: view ? (teamItem(view) ?? null) : TEAM_DEFAULT,
      };
    default:
      return base;
  }
}

function manageLink(item?: string): NavLink {
  const slug = item ? slugOf(MANAGE_SURFACES, item) : undefined;
  if (slug === "identities") return { to: "/manage/identities", params: {} };
  if (slug === "ingestion") return { to: "/manage/ingestion", params: {} };
  return slug
    ? { to: "/manage/$surface", params: { surface: slug } }
    : { to: "/manage", params: {} };
}

function directionLink(dir?: string, lens?: string): NavLink {
  if (!dir) return { to: "/explore", params: {} };
  return lens
    ? { to: "/explore/$dir/$lens", params: { dir, lens: lensSlug(lens) } }
    : { to: "/explore/$dir", params: { dir } };
}
