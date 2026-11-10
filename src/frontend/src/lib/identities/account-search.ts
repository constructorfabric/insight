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

/** INVARIANT: mirrors `FILTER_SQL` and the source-segment match in
 * identity-resolution's `infra/identity_evidence.rs`. */
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
