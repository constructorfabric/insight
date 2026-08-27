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
