import type {
  DashboardAccess,
  FolderAccess,
  FolderSummary,
} from "@/api/custom-client";

/** What a caller with this access may do to a dashboard beyond opening, pinning and filing it. */
export interface DashboardAllows {
  /** Edit, rename, duplicate and tag. */
  edit: boolean;
  share: boolean;
  remove: boolean;
}

export function dashboardAllows(access: DashboardAccess): DashboardAllows {
  switch (access) {
    case "owner":
      return { edit: true, share: true, remove: true };
    case "editor":
      return { edit: true, share: true, remove: false };
    case "ownerless":
      return { edit: true, share: false, remove: true };
    case "viewer":
      return { edit: false, share: false, remove: false };
  }
}

/** What a caller with this access may do to a folder beyond opening it. */
export interface FolderAllows {
  /** Rename it, and add or remove the dashboards in it. */
  edit: boolean;
  share: boolean;
  remove: boolean;
}

/** INVARIANT: only an administrator shares, even a folder they own. */
export function folderAllows(
  access: FolderAccess,
  isAdmin: boolean
): FolderAllows {
  switch (access) {
    case "owner":
      return { edit: true, share: isAdmin, remove: true };
    case "editor":
      return { edit: true, share: isAdmin, remove: false };
    case "ownerless":
      return { edit: true, share: false, remove: true };
    case "viewer":
      return { edit: false, share: false, remove: false };
  }
}

/**
 * The folders a dashboard may be added to: those the caller can edit, an
 * ownerless dashboard only in ownerless ones and any other never in one,
 * leaving out the folders already holding it.
 */
export function fileableFolders(
  access: DashboardAccess,
  folders: readonly FolderSummary[],
  holding: readonly { id: string }[]
): FolderSummary[] {
  const ownerless = access === "ownerless";
  const held = new Set(holding.map((folder) => folder.id));

  return folders.filter(
    (folder) =>
      folderAllows(folder.access, true).edit &&
      (folder.access === "ownerless") === ownerless &&
      !held.has(folder.id)
  );
}
