import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Folder as FolderIcon, X } from "lucide-react";
import { useState } from "react";

import type { DashboardAccess, Folder } from "@/api/custom-client";
import { FieldSelect } from "@/components/alerts/field-select";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { refusal } from "@/components/custom/refusal";
import { Badge } from "@/components/ui/badge";
import { toast } from "@/components/ui/sonner";
import { fileableFolders, folderAllows } from "@/lib/custom/dashboard-access";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { useIsAdmin } from "@/queries/identity-me";
import {
  dashboardReadQuery,
  foldersQuery,
  grantsQuery,
  useAddToFolder,
  useRemoveFromFolder,
} from "@/queries/custom";

/** The folders holding a dashboard, each removable where the caller edits that folder. */
export function DashboardFolders({
  id,
  folders,
}: {
  id: string;
  folders: readonly Folder[];
}) {
  const { isAdmin } = useIsAdmin();
  const listed = useQuery({ ...foldersQuery(), enabled: folders.length > 0 });
  const remove = useRemoveFromFolder();

  if (folders.length === 0) return null;

  const access = new Map(
    listed.data?.folders.map((folder) => [folder.id, folder.access])
  );

  return (
    <ul aria-label="Folders" className="flex flex-wrap items-center gap-1">
      {folders.map((folder) => {
        const held = access.get(folder.id);
        const editable = held ? folderAllows(held, isAdmin).edit : false;

        return (
          <li key={folder.id}>
            <Badge variant="secondary" className="gap-1">
              <FolderIcon aria-hidden />
              <Link to="/portal/custom" search={{ folder: folder.id }}>
                {folder.name}
              </Link>
              {editable ? (
                <button
                  type="button"
                  aria-label={`Remove from ${folder.name}`}
                  disabled={remove.isPending}
                  className="-me-1 inline-flex size-6 items-center justify-center rounded-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() =>
                    remove.mutate(
                      { folder: folder.id, dashboard: id },
                      {
                        onError: (error) =>
                          toast.error(
                            refusal(
                              error,
                              "Couldn't remove it from the folder."
                            )
                          ),
                      }
                    )
                  }
                >
                  <X className="size-3.5" aria-hidden />
                </button>
              ) : null}
            </Badge>
          </li>
        );
      })}
    </ul>
  );
}

/** Adds a dashboard to one more folder the caller edits. */
export function AddToFolder({
  id,
  name,
  access,
  open,
  onClose,
}: {
  id: string;
  name: string;
  access: DashboardAccess;
  open: boolean;
  onClose: () => void;
}) {
  const folders = useQuery({ ...foldersQuery(), enabled: open });
  const read = useQuery({ ...dashboardReadQuery(id), enabled: open });
  const add = useAddToFolder();
  const [picked, setPicked] = useState("");

  const offered = fileableFolders(
    access,
    folders.data?.folders ?? [],
    read.data?.folders ?? []
  );
  const chosen = offered.find((folder) => folder.id === picked);
  const ready = folders.isSuccess && read.isSuccess;

  function close() {
    setPicked("");
    add.reset();
    onClose();
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title={`Add ${name} to a folder`}
      confirmLabel="Add"
      isPending={add.isPending}
      confirmDisabled={!chosen}
      error={
        add.isError
          ? refusal(add.error, "Couldn't add it to the folder.")
          : null
      }
      onConfirm={() => {
        if (!chosen) return;
        add.mutate({ folder: chosen.id, dashboard: id }, { onSuccess: close });
      }}
    >
      {!ready ? null : offered.length === 0 ? (
        <p className={cn(TEXT_BODY, "text-muted-foreground")}>
          No folders to add it to.
        </p>
      ) : (
        <FieldSelect
          id="add-to-folder"
          label="Folder"
          value={picked}
          placeholder="Folder"
          options={offered.map((folder) => ({
            value: folder.id,
            label: folder.name,
          }))}
          onChange={setPicked}
        />
      )}
      {/* INVARIANT: an ownerless folder cannot be shared, so it has no one to warn about. */}
      {chosen && chosen.access !== "ownerless" && read.data?.carries ? (
        <FolderAudience folderId={chosen.id} folderName={chosen.name} />
      ) : null}
    </ConfirmDialog>
  );
}

/** Warns that a folder shared with people opens the dashboard to them too. */
function FolderAudience({
  folderId,
  folderName,
}: {
  folderId: string;
  folderName: string;
}) {
  const grants = useQuery(grantsQuery({ kind: "folders", id: folderId }));

  if (!grants.data || grants.data.length === 0) return null;

  return (
    <p role="status" className={cn(TEXT_BODY, "text-muted-foreground")}>
      People with access to {folderName} will see this dashboard.
    </p>
  );
}
