import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";

/**
 * Empty body for a dashboard group card. `min-h` is the footprint floor so an
 * empty card holds a populated card's height even when its grid row has no
 * taller sibling to stretch against. It fills and centers within the card's
 * content region (the parent `CardContent` carries `flex-1`); no padding of
 * its own, so nothing biases the centered message downward.
 */
export function GroupCardEmpty() {
  return (
    <Empty className="min-h-28 gap-1 p-0">
      <EmptyHeader>
        <EmptyTitle className="text-sm">No data</EmptyTitle>
        <EmptyDescription className="text-xs">
          No metrics with data for this period.
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}
