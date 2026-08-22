/**
 * What actually happened to one account, from the two records the service
 * keeps.
 *
 * Those reads are append-only logs, not an event stream: they hold rows that
 * changed nothing about this account, and the console used to render every one
 * of them.
 *
 * Two sources of noise, both dropped here:
 *
 * 1. Re-observations. The seed reads the whole connector history every run and
 *    appends a binding row per observation, so an account that never moved
 *    collects one automatic row per sync — same person, no reason. Only a row
 *    that changed who holds the account survives.
 * 2. The call. One operator request is journalled once and then returned on
 *    every account it named, so it sits beside the binding row it produced
 *    saying the same thing twice — and it appears on accounts it changed
 *    nothing about at all (already decided, refused, or an account a merge
 *    merely swept along). Its `accounts_touched` counts the whole call, which
 *    for a merge is every account of the absorbed person: a number about other
 *    accounts, on this account's trail.
 *
 * An operator row always survives, even when the person did not change: that
 * row IS the confirm act, a human deciding the resolver's guess stands, and the
 * trail is what says who is answerable for it.
 *
 * The comment is the one thing no binding row holds — why a human did this — so
 * it is carried across onto the row it explains. Nothing links the two records
 * (see `attribute`), so the join is refused wherever it would be a guess: an
 * unattributed comment is invisible rather than attached to the wrong decision.
 */
import type {
  AccountOperation,
  BindingHistoryEntry,
} from "@/api/identity-client";

export interface TrailEvent {
  /** Stable within one read — a render key, never an identifier. */
  key: string;
  entry: BindingHistoryEntry;
  /** Why a human did this, when the call that did it can be named. */
  comment?: string;
}

/** Newest first, as the trail reads. */
export function accountTrail(
  history: BindingHistoryEntry[],
  operations: AccountOperation[] | undefined,
): TrailEvent[] {
  const moved = movements(history);
  const comments = attribute(moved, operations ?? []);
  return moved
    .map((entry, index) => {
      const comment = comments.get(entry);
      return {
        key: `${entry.recorded_at}-${index}`,
        entry,
        ...(comment ? { comment } : {}),
      };
    })
    .reverse();
}

/**
 * The rows that changed something, oldest first.
 *
 * INVARIANT: compared against the previously SEEN person, not the previously
 * kept one. A dropped row is a re-observation of the person it names, so the
 * two hold the same value — and reading the kept one would resurrect a
 * duplicate after every operator row.
 */
function movements(history: BindingHistoryEntry[]): BindingHistoryEntry[] {
  const oldestFirst = [...history].reverse();
  const kept: BindingHistoryEntry[] = [];
  let held: string | undefined;
  for (const entry of oldestFirst) {
    if (entry.by_operator || entry.person_id !== held) kept.push(entry);
    held = entry.person_id;
  }
  return kept;
}

/**
 * Which call explains which decision.
 *
 * Nothing links them: a binding row carries no operation id, and the two
 * timestamps come from different clocks — the app's for the binding, the
 * database's for the call, written afterwards and re-stamped later still on a
 * retry. What the two records DO share is the author and the verb: an operator
 * binding row's `reason` is the same literal the call stores as its `verb`.
 *
 * So the pairing is per (author, verb) group, and only where it is not a guess.
 * One call writes one binding row for this account, so equal counts mean the
 * two sequences are the same calls and pair in time order. Unequal counts mean
 * at least one call wrote nothing here — it was already decided, or refused, or
 * it fell outside the service's cap on how many calls it returns — and then no
 * row in that group gets a comment rather than the wrong one.
 */
function attribute(
  moved: BindingHistoryEntry[],
  operations: AccountOperation[],
): Map<BindingHistoryEntry, string> {
  const named = new Map<BindingHistoryEntry, string>();
  const decisions = groupBy(
    moved.filter((entry) => entry.by_operator),
    (entry) => group(entry.author_person_id, entry.reason),
  );
  const calls = groupBy(operations, (op) => group(op.author_person_id, op.verb));

  for (const [at, rows] of decisions) {
    const made = calls.get(at);
    if (!made || made.length !== rows.length) continue;
    const ordered = [...made].sort((a, b) =>
      a.recorded_at.localeCompare(b.recorded_at),
    );
    // Both oldest-first, so the nth decision in the group is the nth call.
    rows.forEach((entry, index) => {
      const comment = ordered[index]?.comment?.trim();
      if (comment) named.set(entry, comment);
    });
  }
  return named;
}

function group(author: string, verb: string | null | undefined): string {
  return `${author} ${verb?.trim() ?? ""}`;
}

function groupBy<T>(items: T[], of: (item: T) => string): Map<string, T[]> {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const at = of(item);
    const existing = groups.get(at);
    if (existing) existing.push(item);
    else groups.set(at, [item]);
  }
  return groups;
}
