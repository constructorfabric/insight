interface SearchableAccount {
  source: string;
  account_id: string;
  email?: string | null;
  username?: string | null;
  display_name?: string | null;
}

function anchoredToSegment(value: string): string {
  return `_${value.toLowerCase().replaceAll("-", "_")}`;
}

/**
 * INVARIANT: mirrors the listing's filter in the identity-resolution service
 * (`FILTER_SQL`, `SOURCE_SEGMENTS_SQL`, `NEEDLE_SEGMENT_SQL` in
 * `infra/identity_evidence.rs`) so a narrowed Unmatched block and the listing
 * beneath it answer one typed term the same way.
 */
export function matchesAccountSearch(
  account: SearchableAccount,
  needle: string
): boolean {
  const term = needle.trim().toLowerCase();
  if (term === "") return true;

  const inValue = [
    account.email,
    account.username,
    account.account_id,
    account.display_name,
  ].some((value) => value?.toLowerCase().includes(term));

  return (
    inValue ||
    anchoredToSegment(account.source).includes(anchoredToSegment(term))
  );
}
