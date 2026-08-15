/**
 * The identity-resolution operator console (Manage → Identities): the review
 * queue, and the window one case is decided in.
 *
 * A triage surface, not a roster: the operator lands in what NEEDS a
 * decision, grouped by why it does, and works the backlog to zero — the
 * empty queue is the goal state and renders as one. The rates strip on top
 * counts binding states across the tenant; only its first figure, the queue's
 * own size, is work the operator can do.
 *
 * The queue picks a case; the window decides it. Selection lives in `?acct=`
 * so an operator can hand a colleague a link to the exact account under
 * discussion — and that link answers whatever the queue looks like by then,
 * an emptied backlog included.
 */
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { AttentionItem, ResolutionRates } from "@/api/identity-client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { CopyValueButton } from "@/components/copy-value-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import {
  usePortalSearch,
  useSetPortalSearch,
} from "@/lib/portal/portal-search";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { itemKey, parseAccountKey } from "@/lib/identities/account-key";
import {
  filterQueue,
  groupIntoCases,
  type QueueCase,
} from "@/lib/identities/cases";
import { AccountDetail } from "@/components/portal/account-detail";
import { useAttention } from "@/queries/identity-resolution";
import { TEXT_FIGURE, TEXT_LABEL } from "@/lib/type-scale";
import { STATUS_SURFACE_CLASS, type Status } from "@/lib/status";
import { cn } from "@/lib/utils";
import {
  ChevronDown,
  Info,
  PartyPopper,
  Search,
  TriangleAlert,
} from "lucide-react";

/** Queue groups in working order: conflicts first, then the unknowns. */
const KIND_ORDER = ["contested", "binding_conflict", "no_evidence"] as const;

// Binding states, not workloads: every one of these counts accounts the
// resolver has already placed or will place by itself. The only number an
// operator can act on is the queue's own size, which is why it leads the strip
// and is the only one carrying a status colour.
const RATE_TILES: ReadonlyArray<{ key: keyof ResolutionRates; status: Status }> = [
  { key: "observed", status: "neutral" },
  { key: "bound", status: "good" },
  { key: "pending", status: "neutral" },
  { key: "no_evidence", status: "neutral" },
  { key: "excluded", status: "neutral" },
];

/** A click selects the case — unless it pressed a control or ended a selection. */
function opensTheCase(event: React.MouseEvent<HTMLElement>): boolean {
  if (event.target instanceof Element && event.target.closest("button, a")) {
    return false;
  }
  const selection = window.getSelection();
  return !selection || selection.isCollapsed;
}

export function IdentitiesView() {
  const { t } = useTranslation();
  const attention = useAttention();

  if (attention.isLoading) return <CenteredSpinner className="min-h-[60vh]" />;
  if (attention.isError || !attention.data) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label={t("identities.queue.load_failed")}
          onRetry={() => void attention.refetch()}
        />
      </div>
    );
  }

  const { items, rates, truncated, items_truncated: itemsTruncated } =
    attention.data;
  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
      <header>
        <h1 className="text-lg font-semibold tracking-tight">
          {t("identities.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("identities.subtitle")}
        </p>
      </header>
      {truncated ? (
        <Alert variant="destructive" role="status">
          <TriangleAlert />
          <AlertDescription>{t("identities.queue.truncated")}</AlertDescription>
        </Alert>
      ) : null}
      {/* The list was cut by the server's item cap while the rates stay
          whole-tenant — a different fact from `truncated`, and one an
          operator working to zero has to know. */}
      {itemsTruncated && !truncated ? (
        <Alert role="status">
          <TriangleAlert />
          <AlertDescription>
            {t("identities.queue.items_truncated")}
          </AlertDescription>
        </Alert>
      ) : null}
      <RatesStrip
        rates={rates}
        decisions={items.length}
        decisionsCapped={Boolean(itemsTruncated)}
      />
      <Queue items={items} />
    </div>
  );
}

function RatesStrip({
  rates,
  decisions,
  decisionsCapped,
}: {
  rates: ResolutionRates;
  /** Cases in the queue — the only figure here that is the operator's work. */
  decisions: number;
  /** The server cut the list, so the queue size is a floor, not the total. */
  decisionsCapped: boolean;
}) {
  const { t } = useTranslation();
  return (
    <TooltipProvider>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3">
        <Tile
          figure={decisionsCapped ? `${decisions}+` : String(decisions)}
          label={t("identities.rates.decisions")}
          hint={t("identities.rates.decisions_hint")}
          status="warn"
        />
        {RATE_TILES.map(({ key, status }) => (
          <Tile
            key={key}
            figure={String(rates[key])}
            label={t(`identities.rates.${key}`)}
            hint={t(`identities.rates.${key}_hint`)}
            status={status}
          />
        ))}
      </div>
    </TooltipProvider>
  );
}

function Tile({
  figure,
  label,
  hint,
  status,
}: {
  figure: string;
  label: string;
  hint: string;
  status: Status;
}) {
  return (
    <div className="rounded-lg border bg-card p-4">
      <div className={TEXT_FIGURE}>{figure}</div>
      <span className="mt-1 inline-flex items-center gap-1">
        <span
          className={cn(
            TEXT_LABEL,
            "inline-block rounded px-1.5 py-0.5",
            STATUS_SURFACE_CLASS[status],
          )}
        >
          {label}
        </span>
        <Tooltip>
          <TooltipTrigger
            render={<span className="inline-flex text-muted-foreground" />}
            aria-label={label}
          >
            <Info className="size-3.5" />
          </TooltipTrigger>
          <TooltipContent className="max-w-xs">{hint}</TooltipContent>
        </Tooltip>
      </span>
    </div>
  );
}

/** The goal state, celebrated rather than rendered as a blank table. */
function AllResolved() {
  const { t } = useTranslation();
  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <PartyPopper />
        </EmptyMedia>
        <EmptyTitle>{t("identities.queue.empty_title")}</EmptyTitle>
        <EmptyDescription>
          {t("identities.queue.empty_description")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

function Queue({ items: everything }: { items: AttentionItem[] }) {
  const { t } = useTranslation();
  const { acct, filter } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const items = useMemo(
    () => filterQueue(everything, filter ?? ""),
    [everything, filter],
  );
  const groups: Array<{ kind: string; items: AttentionItem[] }> = KIND_ORDER.map(
    (kind) => ({ kind, items: items.filter((i) => i.kind === kind) }),
  ).filter((g) => g.items.length > 0);
  // An unknown kind from a newer backend still shows up rather than
  // vanishing — the vocabulary is open by contract.
  const known = new Set<string>(KIND_ORDER);
  const other = items.filter((i) => !known.has(i.kind));
  if (other.length > 0) groups.push({ kind: "other", items: other });

  // The worked-to-zero queue is the goal state — but a shared `?acct=` link
  // has to answer even then, and the backlog reaching zero is exactly when a
  // colleague opens the link they were sent.
  return (
    <div className="flex min-w-0 flex-col gap-4">
      {everything.length > 0 ? <QueueFilter /> : null}
      {/* A filter that matches nothing is not an emptied backlog. Celebrating
          there would tell an operator the work is done because they mistyped. */}
      {items.length === 0 && everything.length > 0 ? (
        <Empty className="rounded-lg border">
          <EmptyHeader>
            <EmptyTitle>{t("identities.queue.no_matches")}</EmptyTitle>
            <EmptyDescription>
              {t("identities.queue.no_matches_description")}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : null}
      {items.length === 0 && everything.length === 0 ? <AllResolved /> : null}
      {groups.map((group) => (
        <QueueGroup
          key={group.kind}
          kind={group.kind}
          items={group.items}
          selectedKey={acct}
          onSelect={(key) => setAcct(key === acct ? null : key)}
        />
      ))}
      {/* Fed from the unfiltered set: a link stays answerable even when the
          reader's own filter hides the row it points at. */}
      <CaseDialog acct={acct} items={everything} onClose={() => setAcct(null)} />
    </div>
  );
}

/**
 * Narrow the queue by anything visible on a row — an address, a source, a
 * candidate's name, a person id pasted back from a card.
 *
 * The query rides in the URL like every other portal state, so a narrowed
 * queue is shareable; it is written on a pause in typing rather than per
 * keystroke, and replaces rather than pushes, so Back leaves the surface
 * instead of walking the operator backwards through their own typing.
 */
function QueueFilter() {
  const { t } = useTranslation();
  const { filter } = usePortalSearch();
  const setSearch = useSetPortalSearch();
  const [query, setQuery] = useState(filter ?? "");
  const debounced = useDebouncedValue(query, FILTER_DEBOUNCE_MS);

  useEffect(() => {
    setSearch({ filter: debounced.trim() || undefined }, { replace: true });
  }, [debounced, setSearch]);

  return (
    <div className="relative">
      <Search className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        type="search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder={t("identities.queue.filter_placeholder")}
        aria-label={t("identities.queue.filter_placeholder")}
        className="ps-9"
      />
    </div>
  );
}

/**
 * One account under review, in a window rather than a column: this is where
 * every decision is taken, and a decision that re-attributes a person's work
 * deserves the room to show what it acts on.
 *
 * Opened by the `?acct=` in the URL — never by click state alone — so a link
 * an operator shares lands their colleague on the same case.
 */
function CaseDialog({
  acct,
  items,
  onClose,
}: {
  acct: string | undefined;
  items: AttentionItem[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const ref = parseAccountKey(acct);
  const queueItem = items.find((i) => itemKey(i) === acct);
  const heading =
    queueItem?.email?.trim() || queueItem?.username?.trim() || ref?.account_id;

  return (
    <Dialog
      open={ref != null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="max-h-[85vh] gap-4 overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="truncate select-text">{heading}</DialogTitle>
          <DialogDescription
            render={<div className="flex items-center gap-1" />}
          >
            <span className="truncate font-mono text-xs select-text">
              {ref?.source} · {ref?.account_id}
            </span>
            {ref ? (
              <CopyValueButton
                value={ref.account_id}
                title={t("identities.detail.copy_account_id")}
                copyLabel={t("common.copy")}
                copiedLabel={t("common.copied")}
                errorMessage={t("common.copy_failed")}
              />
            ) : null}
          </DialogDescription>
        </DialogHeader>
        {/* Keyed by the account: the body holds per-account state (a verb's
            outcome, an open confirmation), and a cached binding renders the
            next case synchronously — unkeyed, that state would follow. */}
        {ref ? (
          <AccountDetail key={acct} accountRef={ref} queueItem={queueItem} />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

/** Cases rendered before the group asks to be expanded further. */
const CASE_PAGE = 10;

const FILTER_DEBOUNCE_MS = 250;

function QueueGroup({
  kind,
  items,
  selectedKey,
  onSelect,
}: {
  kind: string;
  items: AttentionItem[];
  selectedKey: string | undefined;
  onSelect: (key: string) => void;
}) {
  const { t } = useTranslation();
  const [shownCases, setShownCases] = useState(CASE_PAGE);
  const cases = useMemo(() => groupIntoCases(items), [items]);
  const visible = cases.slice(0, shownCases);
  const hidden = cases.length - visible.length;

  return (
    <Card className="overflow-hidden">
      <Collapsible defaultOpen>
        <CollapsibleTrigger
          render={
            <button
              type="button"
              className="group flex w-full cursor-pointer items-center gap-2 px-6 py-4 text-start hover:bg-accent/40"
            />
          }
        >
          <ChevronDown className="size-4 shrink-0 text-muted-foreground transition-transform group-data-[panel-open]:rotate-180" />
          <CardTitle className="flex flex-wrap items-center gap-2 text-sm">
            {t(`identities.kind.${kind}`, { defaultValue: kind })}
            <Badge variant="secondary">
              {cases.length === items.length
                ? items.length
                : t("identities.queue.case_count", {
                    count: cases.length,
                    accounts: items.length,
                  })}
            </Badge>
          </CardTitle>
          <SourceCounts items={items} />
        </CollapsibleTrigger>
        <CollapsibleContent>
          <CardContent className="flex flex-col gap-2 p-2 pt-0">
            {visible.map((queueCase) => (
              <CaseBlock
                key={queueCase.key}
                queueCase={queueCase}
                selectedKey={selectedKey}
                onSelect={onSelect}
              />
            ))}
            {hidden > 0 ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="self-start"
                onClick={() => setShownCases((n) => n + CASE_PAGE)}
              >
                {t("identities.queue.show_more", { count: hidden })}
              </Button>
            ) : null}
          </CardContent>
        </CollapsibleContent>
      </Collapsible>
    </Card>
  );
}

/** Which connectors this group's accounts came from, so a glance places it. */
function SourceCounts({ items }: { items: AttentionItem[] }) {
  const counts = new Map<string, number>();
  for (const item of items) {
    counts.set(item.source, (counts.get(item.source) ?? 0) + 1);
  }
  return (
    <span className="ms-auto flex flex-wrap items-center gap-1">
      {[...counts.entries()]
        .sort((a, b) => b[1] - a[1])
        .map(([source, count]) => (
          <span
            key={source}
            className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground"
          >
            {source} {count}
          </span>
        ))}
    </span>
  );
}

/**
 * One argument, however many accounts it spans: the people under discussion
 * once at the top, then the accounts each decision is taken on.
 */
function CaseBlock({
  queueCase,
  selectedKey,
  onSelect,
}: {
  queueCase: QueueCase;
  selectedKey: string | undefined;
  onSelect: (key: string) => void;
}) {
  const { t } = useTranslation();
  const disputed = queueCase.candidates.length > 0;
  return (
    <div className={cn(disputed && "rounded-lg border bg-muted/20 p-2")}>
      {disputed ? (
        <div className="flex flex-col gap-2 p-1">
          <div className="text-xs text-muted-foreground">
            {t("identities.queue.case_summary", {
              people: queueCase.candidates.length,
              count: queueCase.items.length,
            })}
          </div>
          {queueCase.candidates.map((candidate) => (
            <PersonCell key={candidate.person_id} person={candidate} />
          ))}
        </div>
      ) : null}
      <div className="flex flex-col gap-1">
        {queueCase.items.map((item) => {
          const key = itemKey(item);
          const selected = key === selectedKey;
          const label =
            item.email?.trim() || item.username?.trim() || item.account_id;
          return (
            <div
              key={key}
              // Not a <button>: its text is what an operator copies out — an
              // address, an account id, a person id — and a button neither
              // lets that text be selected nor may contain the copy controls
              // a card carries.
              role="button"
              tabIndex={0}
              onClick={(event) => {
                if (opensTheCase(event)) onSelect(key);
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                if (event.target !== event.currentTarget) return;
                event.preventDefault();
                onSelect(key);
              }}
              aria-pressed={selected}
              // Without a label the name is computed from everything inside,
              // which reads out before the account is even named.
              aria-label={`${label} ${item.source}`}
              className={cn(
                "cursor-pointer rounded-md border p-3 text-start select-text",
                selected
                  ? "border-ring bg-muted"
                  : "border-transparent hover:bg-muted/60",
              )}
            >
              <div className="flex items-baseline gap-2">
                <span className="truncate text-sm font-medium">{label}</span>
                <span className="ms-auto shrink-0 font-mono text-xs text-muted-foreground">
                  {item.source}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

