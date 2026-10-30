import {
  Folder,
  LayoutDashboard,
  Search,
  Share2,
  Tag,
  type LucideIcon,
} from "lucide-react";

import type { OwnedFilter } from "@/api/custom-client";

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

const NOTHING_SHARED: EmptyCase = {
  icon: Share2,
  title: "No shared dashboards",
  action: "none",
};

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
      title: "No dashboards match your search and tags",
      action: "clear-filters",
    };
  }
  if (searching) {
    return {
      icon: Search,
      title: "No dashboards match your search",
      action: "clear-search",
    };
  }
  if (tagged) {
    return {
      icon: Tag,
      title: "No dashboards with these tags",
      action: "clear-filters",
    };
  }
  if (owned === "shared") return NOTHING_SHARED;
  if (folder === "unfiled") {
    return {
      icon: Folder,
      title: "Every dashboard is in a folder",
      action: "none",
    };
  }
  if (folder === "named") {
    return { icon: Folder, title: "This folder is empty", action: "none" };
  }
  if (!isAdmin) return NOTHING_SHARED;

  return {
    icon: LayoutDashboard,
    title: "No dashboards yet",
    description: assistant
      ? "Describe one to the assistant and it will build it."
      : undefined,
    action: "create",
  };
}
