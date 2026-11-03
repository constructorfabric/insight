/**
 * Empty body for a dashboard group card. `min-h` is the footprint floor so an
 * empty card holds a populated card's height even when its grid row has no
 * taller sibling to stretch against. It fills and centers within the card's
 * content region (the parent `CardContent` carries `flex-1`); no padding of
 * its own, so nothing biases the centered message downward.
 *
 * `notReady` separates the two emptinesses the way `person-coverage` does:
 * "no metrics with data" is a claim about the period, which a card whose
 * source has never been built cannot make.
 */
export function GroupCardEmpty({ notReady = false }: { notReady?: boolean }) {
  return (
    <div className="flex min-h-28 w-full min-w-0 flex-1 flex-col items-center justify-center text-center text-balance">
      <div className="flex max-w-sm flex-col items-center gap-2">
        <p className="text-sm font-medium tracking-tight">
          {notReady ? "No data yet" : "No data"}
        </p>
        <p className="text-xs/relaxed text-muted-foreground">
          {notReady
            ? "Nothing is collected here yet — this fills in after the connector behind it syncs."
            : "No metrics with data for this period."}
        </p>
      </div>
    </div>
  );
}
