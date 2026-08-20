/**
 * A record's text, linked when the record has a page to link to.
 *
 * Metric evidence is only sometimes addressable — it depends on the provider
 * and on what the row carries (see `lib/metrics/git-links`). Callers pass the
 * href they resolved and this decides nothing except how to render it, so an
 * unlinkable row is plain text rather than a dead anchor.
 */
export function RecordLink({
  href,
  children,
}: {
  href: string | undefined;
  children: string;
}) {
  if (!href) return <>{children}</>;
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      // INVARIANT: rows are clickable in their own right (expand, drilldown) —
      // following a link must not also trigger the row.
      onClick={(event) => event.stopPropagation()}
      className="text-foreground hover:underline"
    >
      {children}
    </a>
  );
}
