import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";

import type {
  OrphanKind,
  OrphanedDashboard,
  OrphanedFolder,
} from "@/api/custom-client";
import type { PersonSummary } from "@/api/identity-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { AdminPicker } from "@/components/custom/admin-picker";
import { MoreDefinitions } from "@/components/custom/definition-paging";
import { PersonName } from "@/components/custom/person-name";
import { refusal } from "@/components/custom/refusal";
import { PersonCell } from "@/components/portal/person-cell";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { formatUtcInstant } from "@/lib/format";
import { TEXT_HEADING, TEXT_TITLE } from "@/lib/type-scale";
import { orphanedPagesQuery, useTransferOrphan } from "@/queries/custom";

/** One orphan as the transfer acts on it. */
interface Orphan {
  kind: OrphanKind;
  id: string;
  name: string;
  owner_person_id: string;
}

const NOUN: Record<OrphanKind, string> = {
  dashboards: "dashboard",
  folders: "folder",
};

export function Orphaned() {
  const [moving, setMoving] = useState<Orphan | null>(null);

  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      <h1 className={TEXT_TITLE}>Orphaned</h1>
      <OrphanSection
        kind="dashboards"
        title="Dashboards"
        onTransfer={setMoving}
      />
      <OrphanSection kind="folders" title="Folders" onTransfer={setMoving} />
      <TransferOrphan orphan={moving} onClose={() => setMoving(null)} />
    </div>
  );
}

function OrphanSection({
  kind,
  title,
  onTransfer,
}: {
  kind: OrphanKind;
  title: string;
  onTransfer: (orphan: Orphan) => void;
}) {
  const pages = useInfiniteQuery(orphanedPagesQuery(kind));
  const orphaned: (OrphanedDashboard | OrphanedFolder)[] | undefined =
    pages.data?.pages.flatMap((page) => page.items);
  const headingId = `orphaned-${kind}`;

  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-2">
      <h2 id={headingId} className={TEXT_HEADING}>
        {title}
      </h2>
      {pages.isPending ? (
        <CenteredSpinner className="min-h-24" />
      ) : pages.isError ? (
        <ComingSoon
          variant="card"
          state="error"
          label={refusal(pages.error, `Couldn't load the orphaned ${kind}.`)}
          onRetry={() => void pages.refetch()}
        />
      ) : !orphaned || orphaned.length === 0 ? (
        <ComingSoon
          variant="card"
          state="empty"
          label={`No orphaned ${kind}`}
        />
      ) : (
        <>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Owner</TableHead>
                {kind === "dashboards" ? (
                  <TableHead>Last change</TableHead>
                ) : null}
                <TableHead className="w-28">
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {orphaned.map((orphan) => (
                <TableRow key={orphan.id}>
                  <TableCell className="max-w-0 truncate">
                    {orphan.name}
                  </TableCell>
                  <TableCell className="max-w-0 truncate">
                    <PersonName personId={orphan.owner_person_id} />
                  </TableCell>
                  {"updated_at" in orphan ? (
                    <TableCell>
                      {formatUtcInstant(orphan.updated_at, "d MMM yyyy")}
                    </TableCell>
                  ) : null}
                  <TableCell className="text-end">
                    <Button
                      variant="outline"
                      size="sm"
                      aria-label={`Transfer ${orphan.name}`}
                      onClick={() =>
                        onTransfer({
                          kind,
                          id: orphan.id,
                          name: orphan.name,
                          owner_person_id: orphan.owner_person_id,
                        })
                      }
                    >
                      Transfer
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          <MoreDefinitions
            hasMore={pages.hasNextPage}
            isFetchingMore={pages.isFetchingNextPage}
            onMore={() => void pages.fetchNextPage()}
          />
        </>
      )}
    </section>
  );
}

function TransferOrphan({
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
