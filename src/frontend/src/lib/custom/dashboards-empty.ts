import {
  Folder,
  LayoutDashboard,
  Search,
  Share2,
  Tag,
  type LucideIcon,
} from "lucide-react";

import type { OwnedFilter } from "@/api/custom-client";
import i18n from "@/i18n";

export type FolderShown = "named" | "unfiled";

export interface EmptyListing {
  owned: OwnedFilter | undefined;
  folder: FolderShown | undefined;
  searching: boolean;
  tagged: boolean;
  isAdmin: boolean;
  assistant: boolean;
}

export type EmptyAction = "create" | "clear-search" | "clear-filters" | "none";

export interface EmptyCase {
  icon: LucideIcon;
  title: string;
  description?: string;
  action: EmptyAction;
}

export function emptyCase({
  owned,
  folder,
  searching,
  tagged,
  isAdmin,
  assistant,
}: EmptyListing): EmptyCase {
  if (searching && tagged) {
    return {
      icon: Search,
      title: i18n.t("dashboards.no_search_tags"),
      action: "clear-filters",
    };
  }
  if (searching) {
    return {
      icon: Search,
      title: i18n.t("dashboards.no_search"),
      action: "clear-search",
    };
  }
  if (tagged) {
    return {
      icon: Tag,
      title: i18n.t("dashboards.no_tags"),
      action: "clear-filters",
    };
  }
  if (owned === "shared") {
    return { icon: Share2, title: i18n.t("dashboards.no_shared"), action: "none" };
  }
  if (folder === "unfiled") {
    return {
      icon: Folder,
      title: i18n.t("dashboards.all_filed"),
      action: "none",
    };
  }
  if (folder === "named") {
    return { icon: Folder, title: i18n.t("dashboards.empty_folder"), action: "none" };
  }
  if (!isAdmin) {
    return { icon: Share2, title: i18n.t("dashboards.no_shared"), action: "none" };
  }

  return {
    icon: LayoutDashboard,
    title: i18n.t("dashboards.none_yet"),
    description: assistant ? i18n.t("dashboards.describe_assistant") : undefined,
    action: "create",
  };
}
