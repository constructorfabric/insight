/**
 * Accounts arguing over the same people are ONE case, however many rows the
 * server sends.
 *
 * A binding conflict flags every account that shares the disputed value, and
 * each of those rows carries the identical candidate list — so a single split
 * person reads as five near-identical rows, and the queue looks like five
 * problems. Worse, the rows are ordered by source, which interleaves them with
 * every other case that happens to have an account in the same connector.
 *
 * The candidate set is what identifies the case: it is exactly "who is being
 * argued over". Accounts with no candidates (nothing to match on) are each
 * their own case — there is no argument to join them by.
 */
import type { AttentionItem, PersonSummary } from "@/api/identity-client";
import { accountKey, itemKey } from "@/lib/identities/account-key";

export interface QueueCase {
  /** Stable within one queue read — a render key, never an identifier. */
  key: string;
  /** The people under discussion, once for the whole case. */
  candidates: PersonSummary[];
  /** Every account the case covers, in the order the server sent them. */
  items: AttentionItem[];
}

/**
 * Kinds the console invents for a row that is NOT a queue item.
 *
 * The accounts and persons modes reuse the case window to show a settled
 * account, so their rows travel in the same shape as a queue row. Naming OUR
 * kinds rather than the server's is what keeps the test safe: the server's
 * vocabulary is open, and a kind this build has never seen is a queue item.
 */
export const KIND_SEARCH_MATCH = "match";
export const KIND_PERSON_MEMBER = "member";

/** Whether this row is a real queue item, and so really leaves the queue. */
export function isQueueItem(kind: string): boolean {
  return kind !== KIND_SEARCH_MATCH && kind !== KIND_PERSON_MEMBER;
}

/**
 * Kinds where an operator's only question is "yes, that is right" — so the whole
 * group can be ratified in one press.
 *
 * A contested account has no single answer to apply, a binding conflict is a
 * disagreement to settle rather than to ratify, and an account with no evidence
 * has no binding to re-assert.
 */
const CONFIRMABLE_KINDS: ReadonlySet<string> = new Set([
  "provisioned_at_login",
  "minted_from_roster",
]);

/** Whether a whole queue group can be confirmed in one press. */
export function groupIsConfirmable(
  kind: string,
  items: AttentionItem[],
): boolean {
  return (
    CONFIRMABLE_KINDS.has(kind) &&
    items.length > 0 &&
    // Every row must name the person it would confirm. A row with no holder has
    // nothing to re-assert, and skipping it silently would make the count lie.
    items.every((item) => Boolean(item.bound_to))
  );
}

function caseKey(item: AttentionItem): string {
  if (item.candidates.length === 0) return `account:${itemKey(item)}`;
  const ids = item.candidates.map((c) => c.person_id).sort();
  return `people:${ids.join("|")}`;
}

/**
 * Drop the rows the server just said are decided.
 *
 * Not a guess about the new state — the correction answers per account, and
 * only `applied` and `already_decided` mean a decision now exists. A `refused`
 * account keeps its row, because it kept its binding. Everything else the
 * decision changed (the other accounts of a settled conflict, the rates)
 * arrives with the refetch this only front-runs, so that an operator working
 * the queue does not keep looking at a row they have already dealt with.
 */
export function dropDecided(
  items: AttentionItem[],
  outcomes: ReadonlyArray<{
    source: string;
    source_id: string;
    account_id: string;
    outcome: string;
  }>,
): AttentionItem[] {
  const decided = new Set(
    outcomes
      .filter((o) => o.outcome === "applied" || o.outcome === "already_decided")
      .map(accountKey),
  );
  if (decided.size === 0) return items;
  return items.filter((item) => !decided.has(accountKey(item)));
}

/** Group one kind's rows into cases, first-seen order kept. */
export function groupIntoCases(items: AttentionItem[]): QueueCase[] {
  const byKey = new Map<string, QueueCase>();
  for (const item of items) {
    const key = caseKey(item);
    const existing = byKey.get(key);
    if (existing) {
      existing.items.push(item);
      continue;
    }
    byKey.set(key, { key, candidates: item.candidates, items: [item] });
  }
  return [...byKey.values()];
}
