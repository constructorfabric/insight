import {
  Bell,
  Database,
  LayoutDashboard,
  Sigma,
  type LucideIcon,
} from "lucide-react";

import { kindFromPath } from "@/lib/custom/editor/route";
import {
  itemHidden,
  itemPlanned,
  navPolicy,
  type InstanceNavPolicy,
} from "@/lib/portal/nav-policy";

const CATALOGUE_ZONE = "custom";

type CatalogueId = "metrics" | "widgets" | "datasets" | "alerts";

interface Catalogue {
  id: CatalogueId;
  to:
    | "/portal/custom/metrics"
    | "/portal/custom/widgets"
    | "/portal/custom/datasets"
    | "/portal/custom/alerts";
  label: string;
  icon: LucideIcon;
}

export interface ShownCatalogue extends Catalogue {
  planned: boolean;
}

const CATALOGUES: readonly Catalogue[] = [
  {
    id: "metrics",
    to: "/portal/custom/metrics",
    label: "Metrics",
    icon: Sigma,
  },
  {
    id: "widgets",
    to: "/portal/custom/widgets",
    label: "Widgets",
    icon: LayoutDashboard,
  },
  {
    id: "datasets",
    to: "/portal/custom/datasets",
    label: "Datasets",
    icon: Database,
  },
  { id: "alerts", to: "/portal/custom/alerts", label: "Alerts", icon: Bell },
];

function catalogueShown(
  id: CatalogueId,
  showPlanned: boolean,
  policy: InstanceNavPolicy
): boolean {
  if (itemHidden(CATALOGUE_ZONE, id, policy)) return false;
  return showPlanned || !itemPlanned(CATALOGUE_ZONE, id, policy);
}

export function visibleCatalogues(
  showPlanned: boolean,
  policy: InstanceNavPolicy = navPolicy()
): ShownCatalogue[] {
  return CATALOGUES.filter(({ id }) =>
    catalogueShown(id, showPlanned, policy)
  ).map((catalogue) => ({
    ...catalogue,
    planned: itemPlanned(CATALOGUE_ZONE, catalogue.id, policy),
  }));
}

function catalogueAt(pathname: string): CatalogueId | undefined {
  const path = pathname.toLowerCase();
  const page = CATALOGUES.find(
    ({ to }) => path === to || path.startsWith(`${to}/`)
  );
  if (page) return page.id;

  const editing = kindFromPath(path, "new") ?? kindFromPath(path, "edit");
  return CATALOGUES.find(({ id }) => id === editing)?.id;
}

export function catalogueWithheldAt(
  pathname: string,
  showPlanned: boolean,
  policy: InstanceNavPolicy = navPolicy()
): boolean {
  const id = catalogueAt(pathname);
  if (!id) return false;
  return !catalogueShown(id, showPlanned, policy);
}
