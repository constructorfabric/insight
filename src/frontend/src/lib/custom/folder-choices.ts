import type {
  DashboardAccess,
  Folder,
  FolderSummary,
} from "@/api/custom-client";
import { fileableFolders, folderAllows } from "@/lib/custom/dashboard-access";

export interface FolderChoice {
  folder: Folder;
  holds: boolean;
  /** Whether the caller may put it in, or take it out. */
  movable: boolean;
  /** Null for a folder the caller cannot list, which they cannot change. */
  summary: FolderSummary | null;
}

/**
 * Every folder a dashboard sits in or may go in, each ticked when it holds the
 * dashboard: one click files it there or takes it out.
 */
export function folderChoices(
  access: DashboardAccess,
  listed: readonly FolderSummary[],
  holding: readonly Folder[],
  isAdmin: boolean
): FolderChoice[] {
  const held = new Set(holding.map((folder) => folder.id));
  const fileable = new Set(
    fileableFolders(access, listed, holding).map((folder) => folder.id)
  );
  const known = new Set(listed.map((folder) => folder.id));

  const offered = listed
    .filter((folder) => held.has(folder.id) || fileable.has(folder.id))
    .map((folder) => ({
      folder,
      holds: held.has(folder.id),
      movable: held.has(folder.id)
        ? folderAllows(folder.access, isAdmin).edit
        : true,
      summary: folder,
    }));
  const unlisted = holding
    .filter((folder) => !known.has(folder.id))
    .map((folder) => ({ folder, holds: true, movable: false, summary: null }));

  return [...offered, ...unlisted];
}
