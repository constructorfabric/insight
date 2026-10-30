import { Link, useRouterState } from "@tanstack/react-router";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import {
  Ellipsis,
  Folder,
  FolderX,
  Pencil,
  Plus,
  Share2,
  Trash2,
} from "lucide-react";
import { useId, useState } from "react";

import { FOLDER_NAME_MAX, type FolderSummary } from "@/api/custom-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import {
  type Orphan,
  OrphanedChip,
  TransferOrphan,
} from "@/components/custom/orphaned";
import { CreateFolderDialog } from "@/components/custom/create-folder-dialog";
import { refusal } from "@/components/custom/refusal";
import { ShareDialog } from "@/components/custom/share-dialog";
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
import { folderAllows, type FolderAllows } from "@/lib/custom/dashboard-access";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { useIsAdmin } from "@/queries/identity-me";
import {
  foldersQuery,
  orphanedPagesQuery,
  useDeleteFolder,
  useRenameFolder,
} from "@/queries/custom";

const LIST_PATH = /^\/portal\/custom\/?$/;

export function FoldersGroup() {
  const { data } = useQuery(foldersQuery());
  const { isAdmin } = useIsAdmin();
  const { folder: shown, tag } = usePortalSearch();
  const onList = useRouterState({
    select: (s) => LIST_PATH.test(s.location.pathname),
  });
  const [editing, setEditing] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [deleting, setDeleting] = useState<FolderSummary | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [sharing, setSharing] = useState<FolderSummary | null>(null);
  // INVARIANT: the share dialog plays a close transition after `sharing` is
  // gone, so it keeps the last folder rather than unmounting mid-transition.
  const [shared, setShared] = useState(sharing);
  if (sharing !== null && sharing !== shared) setShared(sharing);
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
                    <Link
                      to="/portal/custom"
                      search={{ folder: folder.id, tag }}
                    />
                  }
                >
                  <Folder />
                  <span>{folder.name}</span>
                </SidebarMenuButton>
                <CountBadge>{folder.dashboards}</CountBadge>
                <FolderMenu
                  name={folder.name}
                  allows={folderAllows(folder.access, isAdmin)}
                  onRename={() => setEditing(folder.id)}
                  onShare={() => setSharing(folder)}
                  onDelete={() => {
                    setDeleting(folder);
                    setConfirming(true);
                  }}
                />
              </SidebarMenuItem>
            )
          )}
          {isAdmin ? <OrphanedFolderRows /> : null}
          {data ? (
            <SidebarMenuItem>
              <SidebarMenuButton
                isActive={isShown("unfiled")}
                render={
                  <Link
                    to="/portal/custom"
                    search={{ folder: "unfiled", tag }}
                  />
                }
              >
                <Folder />
                <span>Unfiled</span>
              </SidebarMenuButton>
              <CountBadge>{data.unfiled}</CountBadge>
            </SidebarMenuItem>
          ) : null}
          <SidebarMenuItem>
            <SidebarMenuButton onClick={() => setCreating(true)}>
              <Plus />
              <span>New folder</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarGroupContent>
      <CreateFolderDialog open={creating} onClose={() => setCreating(false)} />
      <DeleteFolder
        folder={deleting}
        open={confirming}
        onClose={() => setConfirming(false)}
      />
      {shared ? (
        <ShareDialog
          target={{ kind: "folders", id: shared.id }}
          title={shared.name}
          ownerPersonId={shared.owner_person_id}
          open={sharing !== null}
          onClose={() => setSharing(null)}
        />
      ) : null}
    </SidebarGroup>
  );
}

/** Administrators only: the orphaned list is refused to anyone else. */
function OrphanedFolderRows() {
  const pages = useInfiniteQuery(orphanedPagesQuery("folders"));
  const [moving, setMoving] = useState<Orphan | null>(null);
  const orphaned = pages.data?.pages.flatMap((page) => page.items) ?? [];

  return (
    <>
      {orphaned.map((folder) => (
        <SidebarMenuItem key={folder.id}>
          <SidebarMenuButton
            aria-haspopup="dialog"
            className="text-muted-foreground"
            onClick={() =>
              setMoving({
                kind: "folders",
                id: folder.id,
                name: folder.name,
                owner_person_id: folder.owner_person_id,
              })
            }
          >
            <FolderX />
            <span className="min-w-0 flex-1 truncate">{folder.name}</span>
            <OrphanedChip />
          </SidebarMenuButton>
        </SidebarMenuItem>
      ))}
      {pages.hasNextPage ? (
        <SidebarMenuItem>
          <SidebarMenuButton
            className="text-muted-foreground"
            disabled={pages.isFetchingNextPage}
            onClick={() => void pages.fetchNextPage()}
          >
            <Ellipsis />
            <span>More orphaned folders</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      ) : null}
      <TransferOrphan orphan={moving} onClose={() => setMoving(null)} />
    </>
  );
}

function FolderMenu({
  name,
  allows,
  onRename,
  onShare,
  onDelete,
}: {
  name: string;
  allows: FolderAllows;
  onRename: () => void;
  onShare: () => void;
  onDelete: () => void;
}) {
  if (!allows.edit && !allows.share && !allows.remove) return null;

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
        {allows.edit ? (
          <DropdownMenuItem onClick={onRename}>
            <Pencil />
            Rename
          </DropdownMenuItem>
        ) : null}
        {allows.share ? (
          <DropdownMenuItem onClick={onShare}>
            <Share2 />
            Share…
          </DropdownMenuItem>
        ) : null}
        {allows.remove ? (
          <DropdownMenuItem variant="destructive" onClick={onDelete}>
            <Trash2 />
            Delete
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function FolderNameField({
  initial,
  onSave,
  onDone,
}: {
  initial: string;
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
          onBlur={() => {
            if (saving) return;
            if (value.trim() === "" || value === initial) onDone();
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

function whatStays(count: number): string {
  if (count === 0) return "It holds no dashboards.";
  if (count === 1) return "Its dashboard is not deleted.";
  return "Its dashboards are not deleted.";
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
      description={folder ? whatStays(folder.dashboards) : undefined}
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
