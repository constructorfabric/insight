/**
 * The decision surface for one account: every correction verb, each behind a
 * confirmation, with the server's per-item outcomes shown verbatim.
 *
 * Grammar (mirrors the API):
 * - Bind — attach THIS account to a person. On the current person it is the
 *   "confirm" act: re-asserting an automatic binding records the operator's
 *   decision and clears the queue item.
 * - Merge — declare the currently bound person and a candidate one human;
 *   EVERY account of the absorbed person moves, so the dialog previews the
 *   list before anything happens.
 * - Detach — mint a fresh person for this account.
 * - Exclude — not a person at all (bot / CI).
 *
 * Outcomes are never collapsed into a success toast: `already_decided` and
 * `refused` are real states the operator must see (#2424).
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type {
  AccountBinding,
  CorrectionResponse,
  PersonSummary,
  WireAccountRef,
} from "@/api/identity-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { PersonCell } from "@/components/portal/person-cell";
import { PersonPicker } from "@/components/portal/person-picker";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { AccountRef } from "@/lib/identities/account-key";
import { personDisplayName } from "@/lib/identities/person-display";
import { apiErrorReason } from "@/lib/query-console/api-error";
import {
  useBindAccount,
  useDetachAccount,
  useExcludeAccount,
  useMergePersons,
  usePersonAccounts,
} from "@/queries/identity-resolution";

type PendingAction =
  | { kind: "closed" }
  | { kind: "bind"; person: PersonSummary }
  | { kind: "merge"; target: PersonSummary }
  | { kind: "detach" }
  | { kind: "exclude" };

export function AccountActions({
  accountRef,
  binding,
  candidates,
}: {
  accountRef: AccountRef;
  binding: AccountBinding;
  candidates: PersonSummary[];
}) {
  const { t } = useTranslation();
  const [action, setAction] = useState<PendingAction>({ kind: "closed" });
  const [outcome, setOutcome] = useState<CorrectionResponse | null>(null);

  const bind = useBindAccount();
  const merge = useMergePersons();
  const detach = useDetachAccount();
  const exclude = useExcludeAccount();

  const wireRef: WireAccountRef = {
    source: accountRef.source,
    source_id: accountRef.source_id,
    id: accountRef.account_id,
  };
  const boundId = binding.person_id ?? null;

  const close = () => {
    setAction({ kind: "closed" });
    // A dialog's error belongs to the attempt made in THAT dialog; without a
    // reset the next dialog opens already wearing the previous failure.
    bind.reset();
    merge.reset();
    detach.reset();
    exclude.reset();
  };
  const done = (result: CorrectionResponse) => {
    setOutcome(result);
    close();
  };

  return (
    <div className="flex flex-col gap-4">
      {outcome ? <OutcomeAlert outcome={outcome} /> : null}

      {candidates.length > 0 ? (
        <section>
          <div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("identities.detail.candidates")}
          </div>
          <div className="flex flex-col gap-2">
            {candidates.map((candidate) => {
              const isBound = candidate.person_id === boundId;
              return (
                <div key={candidate.person_id} className="flex items-center gap-2">
                  <PersonCell person={candidate} className="min-w-0 flex-1" />
                  <Button
                    type="button"
                    size="xs"
                    variant={isBound ? "default" : "outline"}
                    onClick={() => setAction({ kind: "bind", person: candidate })}
                  >
                    {isBound
                      ? t("identities.actions.confirm")
                      : t("identities.actions.bind")}
                  </Button>
                  {boundId && !isBound ? (
                    <Button
                      type="button"
                      size="xs"
                      variant="outline"
                      onClick={() => setAction({ kind: "merge", target: candidate })}
                    >
                      {t("identities.actions.merge")}
                    </Button>
                  ) : null}
                </div>
              );
            })}
          </div>
        </section>
      ) : null}

      <section>
        <div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t("identities.actions.assign_other")}
        </div>
        <PersonPicker
          excludeIds={candidates.map((c) => c.person_id)}
          onPick={(person) => setAction({ kind: "bind", person })}
        />
      </section>

      <section className="flex flex-wrap gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => setAction({ kind: "detach" })}
        >
          {t("identities.actions.detach")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="destructive"
          onClick={() => setAction({ kind: "exclude" })}
        >
          {t("identities.actions.exclude")}
        </Button>
      </section>

      {action.kind === "bind" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={
            action.person.person_id === boundId
              ? t("identities.dialogs.confirm_title")
              : t("identities.dialogs.bind_title")
          }
          description={t("identities.dialogs.bind_description", {
            name: personDisplayName(action.person),
          })}
          confirmLabel={
            action.person.person_id === boundId
              ? t("identities.actions.confirm")
              : t("identities.actions.bind")
          }
          isPending={bind.isPending}
          error={
            bind.isError
              ? apiErrorReason(bind.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() =>
            bind.mutate(
              { account: wireRef, person_id: action.person.person_id },
              { onSuccess: done },
            )
          }
        >
          <PersonCell person={action.person} />
        </ConfirmDialog>
      ) : null}

      {action.kind === "merge" ? (
        <MergeDialog
          sourceId={boundId ?? ""}
          target={action.target}
          isPending={merge.isPending}
          error={
            merge.isError
              ? apiErrorReason(merge.error, t("identities.dialogs.failed"))
              : null
          }
          onClose={close}
          onConfirm={() =>
            merge.mutate(
              {
                source_person_id: boundId ?? "",
                target_person_id: action.target.person_id,
              },
              { onSuccess: done },
            )
          }
        />
      ) : null}

      {action.kind === "detach" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={t("identities.dialogs.detach_title")}
          description={t("identities.dialogs.detach_description")}
          confirmLabel={t("identities.actions.detach_confirm")}
          isPending={detach.isPending}
          error={
            detach.isError
              ? apiErrorReason(detach.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() => detach.mutate({ account: wireRef }, { onSuccess: done })}
        />
      ) : null}

      {action.kind === "exclude" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={t("identities.dialogs.exclude_title")}
          description={t("identities.dialogs.exclude_description")}
          confirmLabel={t("identities.actions.exclude_confirm")}
          destructive
          isPending={exclude.isPending}
          error={
            exclude.isError
              ? apiErrorReason(exclude.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() => exclude.mutate({ account: wireRef }, { onSuccess: done })}
        />
      ) : null}
    </div>
  );
}

/** The merge preview: name what moves BEFORE anything happens. */
function MergeDialog({
  sourceId,
  target,
  isPending,
  error,
  onClose,
  onConfirm,
}: {
  sourceId: string;
  target: PersonSummary;
  isPending: boolean;
  error: string | null;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  const owned = usePersonAccounts(sourceId || null);
  const accounts = owned.data?.accounts ?? [];
  return (
    <ConfirmDialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("identities.dialogs.merge_title")}
      description={t("identities.dialogs.merge_description", {
        name: personDisplayName(target),
      })}
      confirmLabel={t("identities.actions.merge_confirm")}
      destructive
      isPending={isPending}
      // The preview IS the consent: a merge confirmed before the list loads
      // (or over a failed load rendering as "0 accounts move") would move
      // accounts the operator never saw named.
      confirmDisabled={owned.data == null}
      error={error}
      onConfirm={onConfirm}
    >
      {owned.isError ? (
        <div className="flex items-center gap-2 text-sm text-destructive">
          <span>{t("identities.dialogs.merge_preview_failed")}</span>
          <Button
            type="button"
            size="xs"
            variant="outline"
            onClick={() => void owned.refetch()}
          >
            {t("common.actions.retry")}
          </Button>
        </div>
      ) : owned.data == null ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.dialogs.merge_preview_loading")}
        </p>
      ) : (
        <div className="text-sm">
          <p>{t("identities.dialogs.merge_preview", { count: accounts.length })}</p>
          <ul className="mt-1.5 flex flex-col gap-1">
            {accounts.slice(0, 5).map((account) => (
              <li
                key={`${account.source}:${account.source_id}:${account.account_id}`}
                className="font-mono text-xs text-muted-foreground"
              >
                {account.source} · {account.email ?? account.username ?? account.account_id}
              </li>
            ))}
          </ul>
          {accounts.length > 5 ? (
            <p className="mt-1 text-xs text-muted-foreground">
              {t("identities.dialogs.merge_preview_more", {
                count: accounts.length - 5,
              })}
            </p>
          ) : null}
        </div>
      )}
    </ConfirmDialog>
  );
}

/** The server's answer, verbatim — three counters, never a bare "done". */
function OutcomeAlert({ outcome }: { outcome: CorrectionResponse }) {
  const { t } = useTranslation();
  const refused = outcome.items.filter((i) => i.outcome === "refused").length;
  return (
    <Alert variant={refused > 0 ? "destructive" : "default"} role="status">
      <AlertTitle className="flex flex-wrap items-center gap-1.5">
        <Badge variant="secondary">
          {t("identities.outcomes.applied", { count: outcome.applied })}
        </Badge>
        {outcome.already_decided > 0 ? (
          <Badge variant="outline">
            {t("identities.outcomes.already_decided", {
              count: outcome.already_decided,
            })}
          </Badge>
        ) : null}
        {refused > 0 ? (
          <Badge variant="secondary" className="bg-destructive/15 text-destructive">
            {t("identities.outcomes.refused", { count: refused })}
          </Badge>
        ) : null}
      </AlertTitle>
      {outcome.new_person_id ? (
        <AlertDescription className="font-mono text-xs">
          {t("identities.outcomes.new_person")} {outcome.new_person_id}
        </AlertDescription>
      ) : null}
    </Alert>
  );
}
