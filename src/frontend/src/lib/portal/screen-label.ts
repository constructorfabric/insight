import i18n from "@/i18n";
import { resolveText } from "@/i18n/localized-text";
import { GROUPS } from "@/lib/insight/groups";
import { MODE_LABELS } from "@/lib/portal/identity-modes";
import {
  DIRECTIONS,
  MANAGE_ITEMS,
  PEOPLE_ITEMS,
  lensBySlug,
  zoneById,
  zoneItems,
} from "@/lib/portal/nav-model";
import {
  aiItem,
  homeItem,
  manageItem,
  reportItem,
  sectionOf,
  teamItem,
} from "@/lib/portal/portal-routes";

const SEPARATOR = " › ";

const DASHBOARD_PAGES: Record<string, string> = {
  dashboards: "Dashboards",
  metrics: "Metrics",
  widgets: "Widgets",
  datasets: "Datasets",
  alerts: "Alerts",
  new: "New",
  edit: "Edit",
};

const MASKED_ITEMS: Record<string, string> = {
  dashboards: "Dashboard",
  metrics: "Metric",
  widgets: "Widget",
  datasets: "Dataset",
  alerts: "Alert",
};

const PERSON_VIEWS: Record<string, string> = {
  personal: "Personal",
};

function itemLabel(zone: string, item: string): string {
  const items =
    zone === "manage" ? MANAGE_ITEMS : zone === "people" ? PEOPLE_ITEMS : zoneItems(zone);
  const label = items.find((candidate) => candidate.id === item)?.label;
  return label === undefined ? item : resolveText(i18n.t, label);
}

function sectionLabel(section: string): string {
  return GROUPS.find((group) => group.id === section)?.title ?? section;
}

function customLabel(detail: string[]): string {
  return detail
    .filter(Boolean)
    .map((segment, i, segments) =>
      segment === ":id"
        ? (MASKED_ITEMS[segments[i - 1]] ?? "Dashboard")
        : (DASHBOARD_PAGES[segment] ?? segment),
    )
    .join(SEPARATOR);
}

function detailLabel(zone: string, detail: string[]): string {
  const [third, fourth] = detail;
  if (zone === "directions") {
    const direction = DIRECTIONS.find((candidate) => candidate.id === third);
    if (!direction) return third;
    if (!fourth) return direction.name;
    return `${direction.name}${SEPARATOR}${lensBySlug(direction, fourth) ?? fourth}`;
  }
  if (zone === "custom") return customLabel(detail);
  const item = itemLabel(zone, third);
  if (zone === "manage" && third === "identities" && fourth) {
    return `${item}${SEPARATOR}${MODE_LABELS[fourth] ?? fourth}`;
  }
  return item;
}

export function screenLabel(path: string): string {
  const [, first, ...rest] = path.split("/");
  const [second, third, fourth] = rest;
  const zone = (id: string) => zoneById(id)!.label;
  const under = (label: string, detail: string | undefined) =>
    detail ? `${label}${SEPARATOR}${detail}` : label;

  switch (first) {
    case "":
    case undefined:
      return "Home";
    case "home":
      return under(zone("overview"), second && itemLabel("overview", homeItem(second) ?? second));
    case "explore": {
      if (second === "ai") return under(zone("aicost"), third && itemLabel("aicost", aiItem(third) ?? third));
      const direction = DIRECTIONS.find((candidate) => candidate.id === second);
      if (!direction) return zone("directions");
      return under(direction.name, third && (lensBySlug(direction, third) ?? third));
    }
    case "people":
      if (third === "team") {
        return under(zone("people"), fourth && itemLabel("people", teamItem(fourth) ?? fourth));
      }
      return under("Person", third && sectionLabel(sectionOf(third)));
    case "reports":
      return under(zone("reports"), second && itemLabel("reports", reportItem(second) ?? second));
    case "manage": {
      const item = second && itemLabel("manage", manageItem(second) ?? second);
      if (second === "identities") return under(under(zone("manage"), item), third && (MODE_LABELS[third] ?? third));
      if (second === "ingestion") return under(under(zone("manage"), item), third);
      return under(zone("manage"), item);
    }
    case "dashboards":
      return under(zone("custom"), customLabel(rest) || undefined);
    case "portal":
    case "ic":
      // INVARIANT: usage rows recorded before pages had paths of their own still name screens this way.
      return legacyScreenLabel(path);
    default:
      return path;
  }
}

function legacyScreenLabel(path: string): string {
  const [, first, second, ...detail] = path.split("/");
  const [third, fourth] = detail;

  if (!first) return "Home";

  if (first === "portal") {
    if (!second) return "Portal";
    const zone = zoneById(second);
    if (!zone) return GROUPS.find((group) => group.id === second)?.title ?? path;
    if (!third) return zone.label;
    return `${zone.label}${SEPARATOR}${detailLabel(second, detail)}`;
  }

  if (first === "ic" && second) {
    if (third === "team") {
      const people = zoneById("people")!.label;
      if (!fourth) return people;
      const item = PEOPLE_ITEMS.find((candidate) => candidate.id === fourth);
      return `${people}${SEPARATOR}${item?.label ?? sectionLabel(fourth)}`;
    }
    const view = PERSON_VIEWS[third ?? ""];
    if (!view) return path;
    const person = `Person${SEPARATOR}${view}`;
    return fourth ? `${person}${SEPARATOR}${sectionLabel(fourth)}` : person;
  }

  return path;
}
