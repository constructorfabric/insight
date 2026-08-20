/**
 * The identity-resolution operator console (Manage → Identities): the review
 * queue, and the window one case is decided in.
 *
 * A triage surface, not a roster: the operator lands in what NEEDS a
 * decision, grouped by why it does, and works the backlog to zero — the
 * empty queue is the goal state and renders as one. The strip on top sizes the
 * tenant — its people, its accounts, and how many of those accounts are the
 * operator's own backlog.
 *
 * One layout for all three modes: the heading, the tabs and the mode's own
 * search stay put, and only the list under them scrolls. A reader working a
 * long list must not lose the field they are typing into, or the tabs that
 * switch what they are looking at.
 *
 * The queue picks a case; the window decides it. Selection lives in `?acct=`
 * so an operator can hand a colleague a link to the exact account under
 * discussion — and that link answers whatever the queue looks like by then,
 * an emptied backlog included.
 */
import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { AttentionItem, ResolutionRates } from "@/api/identity-client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { AccountSearchView } from "@/components/portal/account-search-view";
import { CaseDialog } from "@/components/portal/case-dialog";
import { PersonAccountsView } from "@/components/portal/person-accounts-view";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardTitle } from "@/components/ui/card";
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
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { ScrollToEnds } from "@/components/widgets/scroll-to-ends";
import {
  usePortalSearch,
  useSetPortalSearch,
} from "@/lib/portal/portal-search";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { itemKey } from "@/lib/identities/account-key";
import { MODES, resolveMode } from "@/lib/portal/identity-modes";
import { personDisplayName } from "@/lib/identities/person-display";
import { groupIntoCases, type QueueCase } from "@/lib/identities/cases";
import { useAttention } from "@/queries/identity-resolution";
import { TEXT_FIGURE, TEXT_LABEL } from "@/lib/type-scale";
import { STATUS_SURFACE_CLASS, type Status } from "@/lib/status";
import { cn } from "@/lib/utils";
import { ChevronDown, PartyPopper, TriangleAlert } from "lucide-react";

/** Queue groups in working order: conflicts first, then the unknowns. */
const KIND_ORDER = [
  "contested",
  "binding_conflict",
  "provisioned_at_login",
  "minted_from_roster",
  "no_evidence",
] as const;

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
  const { mode } = usePortalSearch();
  const setSearch = useSetPortalSearch();
  const active: string = resolveMode(mode);

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-6xl flex-1 flex-col gap-6 p-6">
      <header className="shrink-0">
        <h1 className="text-lg font-semibold tracking-tight">
          {t("identities.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("identities.subtitle")}
        </p>
      </header>
      <Tabs
        className="shrink-0"
        value={active}
        // A mode change drops the open account: a case picked in one mode
        // means nothing in another, and carrying it would open a window the
        // list behind it does not contain.
        onValueChange={(next) =>
          setSearch({ mode: String(next), acct: undefined })
        }
      >
        <TabsList>
          {MODES.map((m) => (
            <TabsTrigger key={m} value={m}>
              {t(`identities.modes.${m}`)}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      {active === "person" ? <PersonAccountsView /> : null}
      {active === "accounts" ? <AccountSearchView /> : null}
      {active === "queue" ? <ReviewQueue /> : null}
    </div>
  );
}

function ReviewQueue() {
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
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-6">
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
    <div className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3">
      {/* A backend a release behind this bundle does not carry the total; an
          unknown figure reads as one rather than as the word "undefined". */}
      <Tile
        figure={String(rates.persons ?? "—")}
        label={t("identities.rates.persons")}
        status="neutral"
      />
      <Tile
        figure={String(rates.observed)}
        label={t("identities.rates.observed")}
        status="neutral"
      />
      {/* The one figure here that is the operator's own work, and the only one
          carrying a status colour. */}
      <Tile
        figure={decisionsCapped ? `${decisions}+` : String(decisions)}
        label={t("identities.rates.decisions")}
        status="warn"
      />
      <Tile
        figure={String(rates.excluded)}
        label={t("identities.rates.excluded")}
        status="neutral"
      />
    </div>
  );
}

function Tile({
  figure,
  label,
  status,
}: {
  figure: string;
  label: string;
  status: Status;
}) {
  return (
    <div className="rounded-lg border bg-card p-4">
      <div className={TEXT_FIGURE}>{figure}</div>
      <span
        className={cn(
          TEXT_LABEL,
          "mt-1 inline-block rounded px-1.5 py-0.5",
          STATUS_SURFACE_CLASS[status],
        )}
      >
        {label}
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

function Queue({ items }: { items: AttentionItem[] }) {
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const listRef = useRef<HTMLDivElement>(null);
  // Session-scoped on purpose: "have I looked at this one" is about the sitting
  // an operator is in, not a preference worth outliving it.
  const [visited, setVisited] = useState<ReadonlySet<string>>(new Set());
  const groups: Array<{ kind: string; items: AttentionItem[] }> = KIND_ORDER.map(
    (kind) => ({ kind, items: items.filter((i) => i.kind === kind) }),
  ).filter((g) => g.items.length > 0);
  // An unknown kind from a newer backend still shows up rather than
  // vanishing — the vocabulary is open by contract.
  const known = new Set<string>(KIND_ORDER);
  const other = items.filter((i) => !known.has(i.kind));
  if (other.length > 0) groups.push({ kind: "other", items: other });

  // The rendered order, flattened: what "the next case" means to someone
  // working down the queue, and it must not be re-derived differently here.
  const ordered = groups.flatMap((group) =>
    groupIntoCases(group.items).flatMap((c) => c.items.map(itemKey)),
  );

  const select = (key: string | null) => {
    if (key) setVisited((seen) => new Set(seen).add(key));
    setAcct(key);
  };

  // Closing the window puts the operator back on the row they opened, not at
  // the top of the page — the queue is worked in one pass.
  const returnFocus = (key: string) => {
    const row =
      listRef.current?.querySelector<HTMLElement>(
        `[data-queue-row="${CSS.escape(key)}"]`,
      ) ??
      // The row an operator just decided is pruned by the time the window
      // closes — fall to the top of the list rather than to nowhere.
      listRef.current?.querySelector<HTMLElement>("[data-queue-row]");
    row?.focus();
  };

  // The queue is a list, so it moves like one. Enter and Space open a row;
  // those stay on the row itself.
  const onArrow = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const rows = [
      ...(listRef.current?.querySelectorAll<HTMLElement>("[data-queue-row]") ??
        []),
    ];
    const at = rows.indexOf(document.activeElement as HTMLElement);
    if (at === -1) return;
    const next = rows[at + (event.key === "ArrowDown" ? 1 : -1)];
    if (!next) return;
    event.preventDefault();
    next.focus();
  };

  // The worked-to-zero queue is the goal state — but a shared `?acct=` link
  // has to answer even then, and the backlog reaching zero is exactly when a
  // colleague opens the link they were sent.
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
      {/* Blocks, not a flex column: inside a bounded scroller a flex child
          shrinks to fit, and a group card is `overflow-hidden` — so every group
          would render its heading over a clipped stump of its own rows. */}
      <div
        ref={listRef}
        onKeyDown={onArrow}
        className="min-h-0 min-w-0 flex-1 space-y-4 overflow-y-auto"
      >
        {items.length === 0 ? <AllResolved /> : null}
        {groups.map((group) => (
          <QueueGroup
            key={group.kind}
            kind={group.kind}
            items={group.items}
            selectedKey={acct}
            visited={visited}
            onSelect={(key) => select(key === acct ? null : key)}
          />
        ))}
      </div>
      <ScrollToEnds scroller={listRef} rows={items.length} />
      <CaseDialog
        acct={acct}
        items={items}
        ordered={ordered}
        onSelect={select}
        onClose={() => {
          const opened = acct;
          setAcct(null);
          if (opened) returnFocus(opened);
        }}
      />
    </div>
  );
}

/** Cases rendered before the group asks to be expanded further. */
const CASE_PAGE = 10;

function QueueGroup({
  kind,
  items,
  selectedKey,
  visited,
  onSelect,
}: {
  kind: string;
  items: AttentionItem[];
  selectedKey: string | undefined;
  visited: ReadonlySet<string>;
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
              className="group sticky top-0 z-10 flex w-full cursor-pointer items-center gap-2 bg-card px-6 py-4 text-start hover:bg-accent/40"
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
                visited={visited}
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

/**
 * Employment status, when the source says it is anything but active: an
 * operator asked to resolve a leaver is usually being asked for nothing.
 */
function StatusBadge({ status }: { status?: string | null }) {
  const value = status?.trim();
  if (!value || value.toLowerCase() === "active") return null;
  return (
    <Badge variant="secondary" className="shrink-0 font-normal">
      {value}
    </Badge>
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
  visited,
  onSelect,
}: {
  queueCase: QueueCase;
  selectedKey: string | undefined;
  visited: ReadonlySet<string>;
  onSelect: (key: string) => void;
}) {
  const { t } = useTranslation();
  const disputed = queueCase.candidates.length > 0;
  return (
    <div className={cn(disputed && "rounded-lg border bg-muted/20 p-2")}>
      {disputed ? (
        <div className="flex flex-col gap-2 p-1">
          <div className="text-xs text-muted-foreground">
            {t("identities.queue.case_people", {
              count: queueCase.candidates.length,
            })}
            {" · "}
            {t("identities.queue.case_accounts", {
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
          // An account with nothing to match on has an id for a name, and an
          // id names nobody. When the source described it, the person is the
          // heading and the id moves beside the source, where the other rows
          // carry theirs.
          const label =
            item.email?.trim() ||
            item.username?.trim() ||
            item.display_name?.trim() ||
            item.account_id;
          const description = [
            item.display_name?.trim() === label ? null : item.display_name,
            item.job_title,
            item.department,
            item.manager_email
              ? t("identities.queue.reports_to", { manager: item.manager_email })
              : null,
          ]
            .map((s) => s?.trim())
            .filter(Boolean)
            .join(" · ");
          // Which of the case's candidates holds THIS account: the candidates
          // are stated once for the whole case, so without this the row asks
          // an operator to decide between two people without saying which one
          // they would be taking it from.
          const boundTo = queueCase.candidates.find(
            (c) => c.person_id === item.bound_to,
          );
          return (
            <div
              key={key}
              // Not a <button>: its text is what an operator copies out — an
              // address, an account id, a person id — and a button neither
              // lets that text be selected nor may contain the copy controls
              // a card carries.
              role="button"
              tabIndex={0}
              data-queue-row={key}
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
              // which reads out before the account is even named. With one,
              // it must still carry what a sighted operator sees at a glance:
              // who holds it, its status, what the source says it is.
              aria-label={[
                label,
                item.source,
                boundTo
                  ? t("identities.queue.bound_to", {
                      name: personDisplayName(boundTo),
                    })
                  : null,
                item.status?.trim().toLowerCase() === "active"
                  ? null
                  : item.status,
                description || null,
              ]
                .filter(Boolean)
                .join(", ")}
              className={cn(
                "cursor-pointer rounded-md border p-3 text-start select-text",
                selected
                  ? "border-ring bg-muted"
                  : "border-transparent hover:bg-muted/60",
              )}
            >
              <div className="flex items-baseline gap-2">
                <span
                  className={cn(
                    "truncate text-sm font-medium",
                    visited.has(key) && !selected && "text-muted-foreground",
                  )}
                >
                  {label}
                </span>
                {boundTo ? (
                  <Badge variant="outline" className="shrink-0 font-normal">
                    {t("identities.queue.bound_to", {
                      name: personDisplayName(boundTo),
                    })}
                  </Badge>
                ) : null}
                <StatusBadge status={item.status} />
                <span className="ms-auto shrink-0 font-mono text-xs text-muted-foreground">
                  {label === item.account_id ? item.source : `${item.source} · ${item.account_id}`}
                </span>
              </div>
              {description ? (
                <div className="truncate text-xs text-muted-foreground">
                  {description}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
