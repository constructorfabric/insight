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
