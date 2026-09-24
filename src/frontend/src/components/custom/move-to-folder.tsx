import { useQuery } from "@tanstack/react-query";
import { Ellipsis, FolderPlus } from "lucide-react";
import { useState } from "react";

import { FOLDER_NAME_MAX, type Folder } from "@/api/custom-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { toast } from "@/components/ui/sonner";
import {
  dashboardFolderQuery,
  foldersQuery,
  useCreateFolder,
  useMoveDashboard,
} from "@/queries/custom";

const UNFILED = "unfiled";

export function MoveToFolder({ name }: { name: string }) {
  const [open, setOpen] = useState(false);
  const [naming, setNaming] = useState(false);
  const folders = useQuery({ ...foldersQuery(), enabled: open });
  const current = useQuery({ ...dashboardFolderQuery(name), enabled: open });
  const move = useMoveDashboard();

  const filed = current.data === undefined ? "" : (current.data?.id ?? UNFILED);

  return (
    <>
      <DropdownMenu open={open} onOpenChange={setOpen}>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground"
              aria-label={`More for ${name}`}
              icon={<Ellipsis />}
            />
          }
        />
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuGroup>
            <DropdownMenuLabel>Move to folder</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={filed}
              onValueChange={(value: string) =>
                move.mutate(
                  { name, folder: value === UNFILED ? null : value },
                  {
                    onError: (error) =>
                      toast.error(
                        refusal(error, "The dashboard could not be moved.")
                      ),
                  }
                )
              }
            >
              {folders.data?.folders.map((folder) => (
                <DropdownMenuRadioItem
                  key={folder.id}
                  value={folder.id}
                  closeOnClick
                >
                  {folder.name}
                </DropdownMenuRadioItem>
              ))}
              <DropdownMenuRadioItem value={UNFILED} closeOnClick>
                Unfiled
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => setNaming(true)}>
            <FolderPlus />
            New folder…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <NewFolderAndMove
        name={name}
        open={naming}
        onClose={() => setNaming(false)}
      />
    </>
  );
}

function NewFolderAndMove({
  name,
  open,
  onClose,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
}) {
  const [value, setValue] = useState("");
  const [made, setMade] = useState<Folder | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const create = useCreateFolder();
  const move = useMoveDashboard();

  function close() {
    setValue("");
    setMade(null);
    setError(null);
    onClose();
  }

  async function confirm() {
    setSaving(true);
    try {
      const folder = made ?? (await create.mutateAsync(value));
      setMade(folder);
      await move.mutateAsync({ name, folder: folder.id });
      close();
    } catch (failure) {
      setError(refusal(failure, "The folder could not be made."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title="New folder"
      description={`Makes the folder and moves ${name} into it.`}
      confirmLabel="Create and move"
      isPending={saving}
      error={error}
      onConfirm={() => void confirm()}
    >
      <Input
        autoFocus
        aria-label="Folder name"
        maxLength={FOLDER_NAME_MAX}
        readOnly={saving || made != null}
        value={value}
        onChange={(event) => {
          setValue(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          void confirm();
        }}
      />
    </ConfirmDialog>
  );
}
