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
  onShareInto,
}: {
  id: string;
  access: DashboardAccess;
  onNewFolder: () => void;
  /** Asked instead of filing when filing would show the dashboard to the folder's people. */
  onShareInto: (folder: FolderSummary) => void;
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

  async function widensAudience(folder: FolderSummary): Promise<boolean> {
    if (folder.access === "ownerless" || !read.data?.carries) return false;

    // SAFETY: a folder whose people cannot be read is treated as if it had some.
    return queryClient
      .fetchQuery(grantsQuery({ kind: "folders", id: folder.id }))
      .then((grants) => grants.length > 0)
      .catch(() => true);
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
    settle(folder.id, holds);
    try {
      if (!holds && summary && (await widensAudience(summary))) {
        onShareInto(summary);
        return;
      }

      settle(folder.id, !holds);
      if (holds) {
        await remove.mutateAsync(entry);
      } else {
        await add.mutateAsync(entry);
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
