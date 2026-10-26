import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Globe, X } from "lucide-react";
import { useEffect, useEffectEvent, useState } from "react";

import {
  EVERYONE,
  isDenied,
  type Grant,
  type GrantLevel,
  type GrantTarget,
  type Grantee,
} from "@/api/custom-client";
import { FieldSelect } from "@/components/alerts/field-select";
import { refusal } from "@/components/custom/refusal";
import { PersonPicker } from "@/components/portal/person-picker";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { toast } from "@/components/ui/sonner";
import { Spinner } from "@/components/ui/spinner";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { grantsQuery, useDeleteGrant, usePutGrant } from "@/queries/custom";
import { usePersonName } from "@/queries/person-name";

const LEVELS = [
  { value: "viewer", label: "Viewer" },
  { value: "editor", label: "Editor" },
] as const satisfies readonly { value: GrantLevel; label: string }[];

function isLevel(value: string): value is GrantLevel {
  return LEVELS.some((level) => level.value === value);
}

/** Everyone first, then people in the order they were granted. */
function shownOrder(grants: readonly Grant[]): Grant[] {
  return [...grants].sort(
    (a, b) => Number(b.grantee === EVERYONE) - Number(a.grantee === EVERYONE)
  );
}

export function ShareDialog({
  target,
  title,
  ownerPersonId,
  open,
  onClose,
}: {
  target: GrantTarget;
  title: string;
  ownerPersonId: string | null;
  open: boolean;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const grants = useQuery({ ...grantsQuery(target), enabled: open });
  const put = usePutGrant(target);
  const remove = useDeleteGrant(target);
  const [level, setLevel] = useState<GrantLevel>("viewer");
  const [error, setError] = useState<string | null>(null);
  const changing = put.isPending || remove.isPending;

  const granted = grants.data ?? [];
  const toEveryone = granted.some((grant) => grant.grantee === EVERYONE);
  const excluded = [
    ...(ownerPersonId ? [ownerPersonId] : []),
    ...granted.map((grant) => grant.grantee),
  ];

  function grant(grantee: Grantee, at: GrantLevel) {
    setError(null);
    put.mutate(
      { grantee, level: at },
      {
        onSuccess: closeIfLockedOut,
        onError: (failure) => refused(failure, "Couldn't share."),
      }
    );
  }

  function withdraw(grantee: Grantee) {
    setError(null);
    remove.mutate(grantee, {
      onSuccess: closeIfLockedOut,
      onError: (failure) => refused(failure, "Couldn't remove access."),
    });
  }

  function refused(failure: unknown, fallback: string) {
    if (isDenied(failure)) {
      dismiss(refusal(failure, fallback));
      return;
    }
    setError(refusal(failure, fallback));
  }

  // INVARIANT: runs after the grants re-read, because the grant hooks return
  // that invalidation from onSettled and the mutation awaits it.
  function closeIfLockedOut() {
    const reread = queryClient.getQueryState(grantsQuery(target).queryKey);
    if (isDenied(reread?.error)) dismiss(null);
  }

  function dismiss(notice: string | null) {
    queryClient.removeQueries({ queryKey: grantsQuery(target).queryKey });
    if (notice) toast.error(notice);
    close();
  }

  const onGrantsDenied = useEffectEvent((failure: unknown) =>
    dismiss(refusal(failure, "Couldn't read who it is shared with."))
  );
  useEffect(() => {
    if (open && !changing && isDenied(grants.error)) {
      onGrantsDenied(grants.error);
    }
  }, [open, changing, grants.error]);

  function close() {
    setError(null);
    setLevel("viewer");
    put.reset();
    remove.reset();
    onClose();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Share {title}</DialogTitle>
        </DialogHeader>

        <div className="flex items-center gap-2">
          <FieldSelect
            id="share-level"
            label="Level"
            value={level}
            options={LEVELS}
            className="w-32"
            onChange={(next) => {
              if (isLevel(next)) setLevel(next);
            }}
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            icon={<Globe />}
            disabled={toEveryone || put.isPending}
            onClick={() => grant(EVERYONE, level)}
          >
            Everyone
          </Button>
        </div>
        <PersonPicker
          excludeIds={excluded}
          onPick={(person) => grant(person.person_id, level)}
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
          {grants.isPending || put.isPending ? (
            <Spinner className="size-4" />
          ) : null}
          {grants.isError ? (
            <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
              {refusal(grants.error, "Couldn't read who it is shared with.")}
            </p>
          ) : grants.data && grants.data.length === 0 ? (
            <p className={cn(TEXT_BODY, "text-muted-foreground")}>No one</p>
          ) : (
            <ul className="flex flex-col gap-1">
              {shownOrder(granted).map((one) => (
                <GrantRow
                  key={one.grantee}
                  grant={one}
                  pending={
                    (remove.isPending && remove.variables === one.grantee) ||
                    (put.isPending && put.variables.grantee === one.grantee)
                  }
                  onLevel={(next) => grant(one.grantee, next)}
                  onRemove={() => withdraw(one.grantee)}
                />
              ))}
            </ul>
          )}
        </section>
      </DialogContent>
    </Dialog>
  );
}

function GrantRow({
  grant,
  pending,
  onLevel,
  onRemove,
}: {
  grant: Grant;
  pending: boolean;
  onLevel: (level: GrantLevel) => void;
  onRemove: () => void;
}) {
  const everyone = grant.grantee === EVERYONE;
  const name = usePersonName(everyone ? null : grant.grantee);
  const shown = everyone ? "Everyone" : (name.data ?? grant.grantee);

  return (
    <li className="flex items-center gap-2">
      <span className={cn(TEXT_BODY, "min-w-0 flex-1 truncate")}>
        {!everyone && name.isPending ? null : shown}
      </span>
      <FieldSelect
        id={`grant-level-${grant.grantee}`}
        label={`Level for ${shown}`}
        value={grant.level}
        options={LEVELS}
        disabled={pending}
        className="w-28"
        onChange={(next) => {
          if (isLevel(next) && next !== grant.level) onLevel(next);
        }}
      />
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
