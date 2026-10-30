import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  createFolder,
  FOLDER_NAME_MAX,
  type DashboardVisibility,
  type Grantee,
} from "@/api/custom-client";
import type { PersonSummary } from "@/api/identity-client";
import { refusal } from "@/components/custom/refusal";
import { VisibilityField } from "@/components/custom/visibility-field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { toast } from "@/components/ui/sonner";
import { Spinner } from "@/components/ui/spinner";
import { audienceChosen, granteesFor } from "@/lib/custom/initial-grants";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { grantViewers, invalidateDashboardList } from "@/queries/custom";
import { useIsAdmin } from "@/queries/identity-me";

const CAPTIONS = {
  private: "Only you",
  everyone: "Everyone can view its dashboards · only you can edit",
  people: "People you pick can view its dashboards · only you can edit",
} as const satisfies Record<DashboardVisibility, string>;

const NAME_FIELD_ID = "new-folder-name";
const ERROR_ID = "new-folder-error";

async function createShared({
  name,
  grantees,
}: {
  name: string;
  grantees: Grantee[];
}): Promise<{ shareFailure: { refused: unknown } | null }> {
  const folder = await createFolder(name);
  const shareFailure = await grantViewers(
    { kind: "folders", id: folder.id },
    grantees
  );

  return { shareFailure };
}

export function CreateFolderDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const { isAdmin } = useIsAdmin();
  const create = useMutation({
    mutationFn: createShared,
    onSettled: () => invalidateDashboardList(queryClient),
  });

  const [name, setName] = useState("");
  const [visibility, setVisibility] = useState<DashboardVisibility>("private");
  const [people, setPeople] = useState<PersonSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  const trimmed = name.trim();
  const pending = create.isPending;
  const ready =
    trimmed !== "" && (!isAdmin || audienceChosen(visibility, people));

  function close() {
    setName("");
    setVisibility("private");
    setPeople([]);
    setError(null);
    create.reset();
    onClose();
  }

  function requestClose() {
    if (pending) return;
    close();
  }

  async function submit() {
    if (!ready || pending) return;

    setError(null);
    try {
      const made = await create.mutateAsync({
        name: trimmed,
        grantees: isAdmin ? granteesFor(visibility, people) : [],
      });
      close();
      if (made.shareFailure !== null) {
        toast.error(
          refusal(made.shareFailure.refused, "Created, but couldn't share it.")
        );
      }
    } catch (failure) {
      setError(refusal(failure, "Couldn't create the folder."));
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) requestClose();
      }}
    >
      <DialogContent className="sm:max-w-[520px]" showCloseButton={!pending}>
        <DialogHeader>
          <DialogTitle>New folder</DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-5">
          <div className="flex flex-col gap-2">
            <Label htmlFor={NAME_FIELD_ID}>Name</Label>
            <Input
              id={NAME_FIELD_ID}
              autoFocus
              aria-invalid={error !== null}
              aria-describedby={error ? ERROR_ID : undefined}
              required
              maxLength={FOLDER_NAME_MAX}
              readOnly={pending}
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                event.preventDefault();
                void submit();
              }}
            />
          </div>

          {isAdmin ? (
            <VisibilityField
              name="new-folder-visibility"
              captions={CAPTIONS}
              value={visibility}
              people={people}
              disabled={pending}
              onChange={setVisibility}
              onPeopleChange={setPeople}
            />
          ) : null}

          {error ? (
            <p
              id={ERROR_ID}
              role="alert"
              className={cn(TEXT_BODY, "text-destructive")}
            >
              {error}
            </p>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              disabled={pending}
              onClick={requestClose}
            >
              Cancel
            </Button>
            <Button
              type="button"
              disabled={!ready || pending}
              onClick={() => void submit()}
            >
              {pending ? <Spinner className="size-3" /> : null}
              Create
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}
