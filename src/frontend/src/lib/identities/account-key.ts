/**
 * The `?acct=` URL key for one source-native account. An account is a triple
 * (source, source_id, account_id); the key packs it into one URL-safe string
 * so an operator can share a link to the exact account they are looking at.
 * Each part is URI-encoded before joining, so an `account_id` containing the
 * separator cannot forge a different triple.
 */
import type { AttentionItem } from "@/api/identity-client";

const SEPARATOR = ":";

export interface AccountRef {
  source: string;
  source_id: string;
  account_id: string;
}

export function accountKey(ref: AccountRef): string {
  return [ref.source, ref.source_id, ref.account_id]
    .map(encodeURIComponent)
    .join(SEPARATOR);
}

/** Inverse of {@link accountKey}; a malformed key reads as "nothing selected". */
export function parseAccountKey(key: string | undefined): AccountRef | null {
  if (!key) return null;
  const parts = key.split(SEPARATOR);
  if (parts.length !== 3 || parts.some((p) => p === "")) return null;
  try {
    const [source, source_id, account_id] = parts.map(decodeURIComponent);
    return { source, source_id, account_id };
  } catch {
    // A truncated %-escape (a link cut mid-character) throws URIError; that is
    // still a malformed key, not a render crash.
    return null;
  }
}

export function itemKey(item: AttentionItem): string {
  return accountKey(item);
}
