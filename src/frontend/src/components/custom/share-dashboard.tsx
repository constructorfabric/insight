import { useQuery } from "@tanstack/react-query";
import { X } from "lucide-react";
import { useState } from "react";

import { AdminPicker } from "@/components/custom/admin-picker";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import {
  sharesQuery,
  useShareDashboard,
  useUnshareDashboard,
} from "@/queries/custom";
import { usePersonName } from "@/queries/person-name";

export function ShareDashboard({
  id,
  title,
  ownerPersonId,
  open,
  onClose,
}: {
  id: string;
  title: string;
  ownerPersonId: string;
  open: boolean;
  onClose: () => void;
}) {
  const shares = useQuery({ ...sharesQuery(id), enabled: open });
  const share = useShareDashboard(id);
  const unshare = useUnshareDashboard(id);
  const [error, setError] = useState<string | null>(null);

  const sharedWith = shares.data?.map((one) => one.person_id) ?? [];

  function refuse(fallback: string) {
    return (failure: unknown) => setError(refusal(failure, fallback));
  }

  function close() {
    setError(null);
    share.reset();
    unshare.reset();
    onClose();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Share {title}</DialogTitle>
        </DialogHeader>

        <AdminPicker
          exclude={[ownerPersonId, ...sharedWith]}
          onPick={(person) => {
            setError(null);
            share.mutate(person.person_id, {
              onError: refuse("The dashboard could not be shared."),
            });
          }}
        />

        {error ? (
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            {error}
          </p>
        ) : null}

        <section aria-labelledby="shared-with" className="flex flex-col gap-1">
          <h3
            id="shared-with"
            className={cn(TEXT_LABEL, "font-medium text-muted-foreground")}
          >
            Shared with
          </h3>
          {shares.isPending || share.isPending ? (
            <Spinner className="size-4" />
          ) : null}
          {shares.isError ? (
            <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
              {refusal(shares.error, "Couldn't read who it is shared with.")}
            </p>
          ) : shares.data && shares.data.length === 0 ? (
            <p className={cn(TEXT_BODY, "text-muted-foreground")}>No one</p>
          ) : (
            <ul className="flex flex-col gap-1">
              {shares.data?.map((one) => (
                <ShareRow
                  key={one.person_id}
                  personId={one.person_id}
                  pending={
                    unshare.isPending && unshare.variables === one.person_id
                  }
                  onRemove={() => {
                    setError(null);
                    unshare.mutate(one.person_id, {
                      onError: refuse("The share could not be removed."),
                    });
                  }}
                />
              ))}
            </ul>
          )}
        </section>
      </DialogContent>
    </Dialog>
  );
}

function ShareRow({
  personId,
  pending,
  onRemove,
}: {
  personId: string;
  pending: boolean;
  onRemove: () => void;
}) {
  const name = usePersonName(personId);
  const shown = name.data ?? personId;

  return (
    <li className="flex items-center gap-2">
      <span className={cn(TEXT_BODY, "min-w-0 flex-1 truncate")}>
        {name.isPending ? null : shown}
      </span>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={`Stop sharing with ${shown}`}
        disabled={pending}
        icon={<X />}
        onClick={onRemove}
      />
    </li>
  );
}
