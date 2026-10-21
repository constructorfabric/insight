import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { OrphanedDashboard } from "@/api/custom-client";
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
import { TEXT_TITLE } from "@/lib/type-scale";
import { orphanedPagesQuery, useTransferDashboard } from "@/queries/custom";

export function OrphanedDashboards() {
  const pages = useInfiniteQuery(orphanedPagesQuery());
  const [moving, setMoving] = useState<OrphanedDashboard | null>(null);
  const orphaned = pages.data?.pages.flatMap((page) => page.items);

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <h1 className={TEXT_TITLE}>Orphaned dashboards</h1>

      {pages.isPending ? (
        <CenteredSpinner className="min-h-40" />
      ) : pages.isError ? (
        <ComingSoon
          variant="card"
          state="error"
          label={refusal(pages.error, "Couldn't load the orphaned dashboards.")}
          onRetry={() => void pages.refetch()}
        />
      ) : !orphaned || orphaned.length === 0 ? (
        <ComingSoon variant="card" state="empty" label="No orphaned dashboards" />
      ) : (
        <>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Owner</TableHead>
                <TableHead>Last change</TableHead>
                <TableHead className="w-28">
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {orphaned.map((dashboard) => (
                <TableRow key={dashboard.id}>
                  <TableCell className="max-w-0 truncate">
                    {dashboard.name}
                  </TableCell>
                  <TableCell className="max-w-0 truncate">
                    <PersonName personId={dashboard.owner_person_id} />
                  </TableCell>
                  <TableCell>
                    {formatUtcInstant(dashboard.updated_at, "d MMM yyyy")}
                  </TableCell>
                  <TableCell className="text-end">
                    <Button
                      variant="outline"
                      size="sm"
                      aria-label={`Transfer ${dashboard.name}`}
                      onClick={() => setMoving(dashboard)}
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

      <TransferDashboard
        dashboard={moving}
        onClose={() => setMoving(null)}
      />
    </div>
  );
}

function TransferDashboard({
  dashboard,
  onClose,
}: {
  dashboard: OrphanedDashboard | null;
  onClose: () => void;
}) {
  const transfer = useTransferDashboard();
  const [to, setTo] = useState<PersonSummary | null>(null);

  function close() {
    setTo(null);
    transfer.reset();
    onClose();
  }

  return (
    <ConfirmDialog
      open={dashboard !== null}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title={`Transfer ${dashboard?.name ?? ""}`}
      confirmLabel="Transfer"
      isPending={transfer.isPending}
      confirmDisabled={to === null}
      error={
        transfer.isError
          ? refusal(transfer.error, "The dashboard could not be transferred.")
          : null
      }
      onConfirm={() => {
        if (!dashboard || !to) return;
        transfer.mutate(
          { id: dashboard.id, to: to.person_id },
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
          exclude={dashboard ? [dashboard.owner_person_id] : []}
          onPick={setTo}
        />
      )}
    </ConfirmDialog>
  );
}
