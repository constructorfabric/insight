import { Link, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Ellipsis, Folder, Inbox, Pencil, Plus, Trash2 } from "lucide-react";
import { useId, useState } from "react";

import { FOLDER_NAME_MAX, type FolderSummary } from "@/api/custom-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { refusal } from "@/components/custom/refusal";
import { CountBadge } from "@/components/portal/pane-nav";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { usePortalSearch } from "@/lib/portal/portal-search";
import {
  foldersQuery,
  useCreateFolder,
  useDeleteFolder,
  useRenameFolder,
} from "@/queries/custom";

const LIST_PATH = /^\/portal\/custom\/?$/;
const NEW_FOLDER = "new";

export function FoldersGroup() {
  const { data } = useQuery(foldersQuery());
  const { folder: shown } = usePortalSearch();
  const onList = useRouterState({
    select: (s) => LIST_PATH.test(s.location.pathname),
  });
  const [editing, setEditing] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<FolderSummary | null>(null);
  const [confirming, setConfirming] = useState(false);
  const create = useCreateFolder();
  const rename = useRenameFolder();

  const isShown = (id: string) => onList && shown === id;

  return (
    <SidebarGroup>
      <SidebarGroupLabel>Folders</SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {data?.folders.map((folder) =>
            editing === folder.id ? (
              <FolderNameField
                key={folder.id}
                initial={folder.name}
                onSave={(name) => rename.mutateAsync({ id: folder.id, name })}
                onDone={() => setEditing(null)}
              />
            ) : (
              <SidebarMenuItem key={folder.id}>
                <SidebarMenuButton
                  isActive={isShown(folder.id)}
                  className="group-has-data-[sidebar=menu-action]/menu-item:pe-14"
                  render={
                    <Link to="/portal/custom" search={{ folder: folder.id }} />
                  }
                >
                  <Folder />
                  <span>{folder.name}</span>
                </SidebarMenuButton>
                <CountBadge>{folder.dashboards}</CountBadge>
                <FolderMenu
                  name={folder.name}
                  onRename={() => setEditing(folder.id)}
                  onDelete={() => {
                    setDeleting(folder);
                    setConfirming(true);
                  }}
                />
              </SidebarMenuItem>
            )
          )}
          {data ? (
            <SidebarMenuItem>
              <SidebarMenuButton
                isActive={isShown("unfiled")}
                render={
                  <Link to="/portal/custom" search={{ folder: "unfiled" }} />
                }
              >
                <Inbox />
                <span>Unfiled</span>
              </SidebarMenuButton>
              <CountBadge>{data.unfiled}</CountBadge>
            </SidebarMenuItem>
          ) : null}
          {editing === NEW_FOLDER ? (
            <FolderNameField
              onSave={(name) => create.mutateAsync(name)}
              onDone={() => setEditing(null)}
            />
          ) : (
            <SidebarMenuItem>
              <SidebarMenuButton
                className="text-muted-foreground"
                onClick={() => setEditing(NEW_FOLDER)}
              >
                <Plus />
                <span>New folder</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          )}
        </SidebarMenu>
      </SidebarGroupContent>
      <DeleteFolder
        folder={deleting}
        open={confirming}
        onClose={() => setConfirming(false)}
      />
    </SidebarGroup>
  );
}

function FolderMenu({
  name,
  onRename,
  onDelete,
}: {
  name: string;
  onRename: () => void;
  onDelete: () => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <SidebarMenuAction
            showOnHover
            className="end-7"
            aria-label={`More for ${name}`}
          />
        }
      >
        <Ellipsis />
      </DropdownMenuTrigger>
      <DropdownMenuContent side="right" align="start" finalFocus={false}>
        <DropdownMenuItem onClick={onRename}>
          <Pencil />
          Rename
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onClick={onDelete}>
          <Trash2 />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function FolderNameField({
  initial = "",
  onSave,
  onDone,
}: {
  initial?: string;
  onSave: (name: string) => Promise<unknown>;
  onDone: () => void;
}) {
  const errorId = useId();
  const [value, setValue] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function save() {
    setSaving(true);
    try {
      await onSave(value);
      onDone();
    } catch (failure) {
      setError(refusal(failure, "The folder could not be saved."));
      setSaving(false);
    }
  }

  return (
    <SidebarMenuItem>
      <form
        className="px-1"
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <Input
          autoFocus
          aria-label="Folder name"
          aria-invalid={error != null}
          aria-describedby={error ? errorId : undefined}
          className="h-8"
          maxLength={FOLDER_NAME_MAX}
          readOnly={saving}
          value={value}
          onChange={(event) => {
            setValue(event.target.value);
            setError(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") onDone();
          }}
        />
        {error ? (
          <p
            id={errorId}
            role="alert"
            className="px-1 pt-1 text-xs text-destructive"
          >
            {error}
          </p>
        ) : null}
      </form>
    </SidebarMenuItem>
  );
}

function whatMoves(count: number): string {
  if (count === 0) return "It holds no dashboards.";
  if (count === 1) return "Its 1 dashboard moves to Unfiled.";
  return `Its ${count} dashboards move to Unfiled.`;
}

function DeleteFolder({
  folder,
  open,
  onClose,
}: {
  folder: FolderSummary | null;
  open: boolean;
  onClose: () => void;
}) {
  const remove = useDeleteFolder();

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (next) return;
        remove.reset();
        onClose();
      }}
      title={`Delete “${folder?.name ?? ""}”?`}
      description={folder ? whatMoves(folder.dashboards) : undefined}
      confirmLabel="Delete folder"
      destructive
      isPending={remove.isPending}
      error={
        remove.isError
          ? refusal(remove.error, "The folder could not be deleted.")
          : null
      }
      onConfirm={() => {
        if (folder) remove.mutate(folder.id, { onSuccess: onClose });
      }}
    />
  );
}
