import type { DashboardAccess } from "@/api/custom-client";

/** What a caller with this access may do to a dashboard beyond opening, pinning and filing it. */
export interface DashboardAllows {
  /** Edit, rename, duplicate, delete and tag. */
  change: boolean;
  share: boolean;
}

export function dashboardAllows(access: DashboardAccess): DashboardAllows {
  switch (access) {
    case "owner":
      return { change: true, share: true };
    case "ownerless":
      return { change: true, share: false };
    case "viewer":
      return { change: false, share: false };
  }
}

/**
 * The folders a dashboard may be filed in: an ownerless one only in ownerless
 * folders, any other only in the caller's own.
 */
export function fileableFolders<T extends { owner_person_id: string | null }>(
  access: DashboardAccess,
  folders: readonly T[]
): T[] {
  const ownerless = access === "ownerless";

  return folders.filter(
    (folder) => (folder.owner_person_id === null) === ownerless
  );
}
