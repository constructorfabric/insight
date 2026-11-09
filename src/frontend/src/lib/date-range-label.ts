function parseIso(iso: string): Date {
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(y ?? 1970, (m ?? 1) - 1, d ?? 1);
}

/**
 * The active range, with the year shown only when it is in doubt.
 *
 * "31 Jul – 30 Jul" for a whole year read as a typo, or as a week, to everyone
 * who saw it: the reader has to already know the answer to decode the label.
 * The year appears when the range crosses a year boundary or sits outside the
 * current one, and stays out of the way when both ends are plainly this year.
 */
export function formatRange(fromIso: string, toIso: string): string {
  const from = parseIso(fromIso);
  const to = parseIso(toIso);
  const thisYear = new Date().getFullYear();
  const needsYear =
    from.getFullYear() !== to.getFullYear() ||
    from.getFullYear() !== thisYear ||
    to.getFullYear() !== thisYear;
  const opts: Intl.DateTimeFormatOptions = needsYear
    ? { day: "numeric", month: "short", year: "numeric" }
    : { day: "numeric", month: "short" };
  return `${from.toLocaleDateString("en-GB", opts)} – ${to.toLocaleDateString("en-GB", opts)}`;
}
