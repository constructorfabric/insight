import { useInfiniteQuery } from "@tanstack/react-query";
import { UserX } from "lucide-react";
import { useState } from "react";

import type { OrphanKind, OrphanedDashboard } from "@/api/custom-client";
import type { PersonSummary } from "@/api/identity-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { AdminPicker } from "@/components/custom/admin-picker";
import { MoreDefinitions } from "@/components/custom/definition-paging";
import { OwnerName } from "@/components/custom/person-name";
import { refusal } from "@/components/custom/refusal";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { formatUtcInstant } from "@/lib/format";
import { TEXT_HEADING, TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { orphanedPagesQuery, useTransferOrphan } from "@/queries/custom";

export interface Orphan {
  kind: OrphanKind;
  id: string;
  name: string;
  owner_person_id: string;
}

const NOUN: Record<OrphanKind, string> = {
  dashboards: "dashboard",
  folders: "folder",
};

const HEADING_ID = "orphaned-dashboards";

export function OrphanedChip() {
  return (
    <Badge variant="warning" size="xs">
      Orphaned
    </Badge>
  );
}

/** Administrators only: the orphaned list is refused to anyone else. */
export function OrphanedDashboards() {
  const pages = useInfiniteQuery(orphanedPagesQuery("dashboards"));
  const [moving, setMoving] = useState<Orphan | null>(null);
  const orphaned = pages.data?.pages.flatMap((page) => page.items) ?? [];

  if (!pages.isError && orphaned.length === 0) return null;

  return (
    <section aria-labelledby={HEADING_ID} className="mt-6 flex flex-col gap-3">
      <h2 id={HEADING_ID} className={TEXT_HEADING}>
        Orphaned
      </h2>
      {pages.isError ? (
        <ComingSoon
          variant="card"
          state="error"
          label={refusal(
            pages.error,
            "Couldn't load the orphaned dashboards."
          )}
          onRetry={() => void pages.refetch()}
        />
      ) : (
        <>
          <ul className="grid gap-3 @xl:grid-cols-2 @5xl:grid-cols-3">
            {orphaned.map((dashboard) => (
              <li key={dashboard.id}>
                <OrphanedDashboardCard
                  dashboard={dashboard}
                  onTransfer={() =>
                    setMoving({
                      kind: "dashboards",
                      id: dashboard.id,
                      name: dashboard.name,
                      owner_person_id: dashboard.owner_person_id,
                    })
                  }
                />
              </li>
            ))}
          </ul>
          <MoreDefinitions
            hasMore={pages.hasNextPage}
            isFetchingMore={pages.isFetchingNextPage}
            onMore={() => void pages.fetchNextPage()}
          />
        </>
      )}
      <TransferOrphan orphan={moving} onClose={() => setMoving(null)} />
    </section>
  );
}

function OrphanedDashboardCard({
  dashboard,
  onTransfer,
}: {
  dashboard: OrphanedDashboard;
  onTransfer: () => void;
}) {
  return (
    <Card size="sm" className="bg-muted/40">
      <CardContent className="flex items-center gap-3">
        <UserX className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className={cn(TEXT_NAME, "truncate text-muted-foreground")}>
            {dashboard.name}
          </span>
          <OwnerName
            personId={dashboard.owner_person_id}
            className={cn(TEXT_LABEL, "truncate font-normal")}
          />
          <span className={cn(TEXT_LABEL, "font-normal")}>
            Changed {formatUtcInstant(dashboard.updated_at, "d MMM yyyy")}
          </span>
        </span>
        <Button
          variant="outline"
          size="sm"
          aria-label={`Transfer ${dashboard.name}`}
          aria-haspopup="dialog"
          onClick={onTransfer}
        >
          Transfer…
        </Button>
      </CardContent>
    </Card>
  );
}

export function TransferOrphan({
  orphan,
  onClose,
}: {
  orphan: Orphan | null;
  onClose: () => void;
}) {
  const transfer = useTransferOrphan();
  const [to, setTo] = useState<PersonSummary | null>(null);
  // INVARIANT: the dialog plays a close transition after `orphan` is gone,
  // so its title keeps the last one rather than shrinking to "Transfer ".
  const [held, setHeld] = useState(orphan);
  if (orphan !== null && orphan !== held) setHeld(orphan);
  const shown = orphan ?? held;

  function close() {
    setTo(null);
    transfer.reset();
    onClose();
  }

  return (
    <ConfirmDialog
      open={orphan !== null}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title={`Transfer ${shown?.name ?? ""}`}
      confirmLabel="Transfer"
      isPending={transfer.isPending}
      confirmDisabled={to === null}
      error={
        transfer.isError
          ? refusal(
              transfer.error,
              `The ${shown ? NOUN[shown.kind] : "item"} could not be transferred.`
            )
          : null
      }
      onConfirm={() => {
        if (!orphan || !to) return;
        transfer.mutate(
          { kind: orphan.kind, id: orphan.id, to: to.person_id },
          { onSuccess: close }
        );
      }}
    >
      {to ? (
        <div className="flex items-center gap-2">
          <PersonCell person={to} className="min-w-0 flex-1" />
          <Button
            variant="ghost"
            size="sm"
            disabled={transfer.isPending}
            onClick={() => {
              setTo(null);
              transfer.reset();
            }}
          >
            Change
          </Button>
        </div>
      ) : (
        <AdminPicker
          exclude={shown ? [shown.owner_person_id] : []}
          onPick={setTo}
        />
      )}
    </ConfirmDialog>
  );
}
