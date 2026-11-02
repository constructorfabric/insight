import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Folder as FolderIcon, Plus } from "lucide-react";
import { useState } from "react";

import type { DashboardAccess, FolderSummary } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import {
  DropdownMenuCheckboxItem,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";
import { toast } from "@/components/ui/sonner";
import { folderChoices, type FolderChoice } from "@/lib/custom/folder-choices";
import { useIsAdmin } from "@/queries/identity-me";
import {
  dashboardReadQuery,
  foldersQuery,
  grantsQuery,
  useAddToFolder,
  useRemoveFromFolder,
} from "@/queries/custom";

export function FolderSubmenu({
  id,
  access,
  onNewFolder,
}: {
  id: string;
  access: DashboardAccess;
  onNewFolder: () => void;
}) {
  const [open, setOpen] = useState(false);
  const { isAdmin } = useIsAdmin();
  const listed = useQuery({ ...foldersQuery(), enabled: open });
  const read = useQuery({ ...dashboardReadQuery(id), enabled: open });
  const add = useAddToFolder();
  const remove = useRemoveFromFolder();
  const queryClient = useQueryClient();

  const ready = listed.isSuccess && read.isSuccess;
  const choices = ready
    ? folderChoices(access, listed.data.folders, read.data.folders, isAdmin)
    : [];
  const [moving, setMoving] = useState<ReadonlyMap<string, boolean>>(new Map());

  async function tellAudience(folder: FolderSummary) {
    if (folder.access === "ownerless" || !read.data?.carries) return;

    // SAFETY: a folder whose people cannot be read is told about as if it had some.
    const shared = await queryClient
      .fetchQuery(grantsQuery({ kind: "folders", id: folder.id }))
      .then((grants) => grants.length > 0)
      .catch(() => true);
    if (shared) {
      toast.success(
        `People with access to ${folder.name} will see this dashboard.`
      );
    }
  }

  function settle(folder: string, holds: boolean | null) {
    setMoving((now) => {
      const next = new Map(now);
      if (holds === null) next.delete(folder);
      else next.set(folder, holds);
      return next;
    });
  }

  // INVARIANT: mutateAsync, not mutate — a second click before the first settles would drop the first call's callbacks.
  async function toggle({ folder, holds, summary }: FolderChoice) {
    if (moving.has(folder.id)) return;

    const entry = { folder: folder.id, dashboard: id };
    settle(folder.id, !holds);
    try {
      if (holds) {
        await remove.mutateAsync(entry);
      } else {
        await add.mutateAsync(entry);
        if (summary) void tellAudience(summary);
      }
    } catch (error) {
      toast.error(
        refusal(
          error,
          holds
            ? "Couldn't remove it from the folder."
            : "Couldn't add it to the folder."
        )
      );
    } finally {
      settle(folder.id, null);
    }
  }

  return (
    <DropdownMenuSub open={open} onOpenChange={setOpen}>
      <DropdownMenuSubTrigger>
        <FolderIcon />
        Folders
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="w-56">
        {!ready ? (
          <DropdownMenuItem disabled>Loading…</DropdownMenuItem>
        ) : choices.length === 0 ? (
          <DropdownMenuItem disabled>No folders yet</DropdownMenuItem>
        ) : (
          choices.map((choice) => (
            <DropdownMenuCheckboxItem
              key={choice.folder.id}
              checked={moving.get(choice.folder.id) ?? choice.holds}
              disabled={!choice.movable}
              onCheckedChange={() => void toggle(choice)}
            >
              {choice.folder.name}
            </DropdownMenuCheckboxItem>
          ))
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={onNewFolder}>
          <Plus />
          New folder…
        </DropdownMenuItem>
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}
