import { NewDashboardButton } from "@/components/custom/create-dashboard-dialog";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyActions,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  emptyCase,
  type EmptyAction,
  type EmptyListing,
} from "@/lib/custom/dashboards-empty";

interface Clearing {
  onClearSearch: () => void;
  onClearFilters: () => void;
}

function ActionFor({
  action,
  onClearSearch,
  onClearFilters,
}: Clearing & { action: EmptyAction }) {
  switch (action) {
    case "create":
      return <NewDashboardButton />;
    case "clear-search":
      return (
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={onClearSearch}
        >
          Clear search
        </Button>
      );
    case "clear-filters":
      return (
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={onClearFilters}
        >
          Clear filters
        </Button>
      );
    case "none":
      return null;
  }
}

export function DashboardsEmpty({
  onClearSearch,
  onClearFilters,
  ...listing
}: EmptyListing & Clearing) {
  const { icon: Icon, title, description, action } = emptyCase(listing);

  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Icon aria-hidden />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        {description ? (
          <EmptyDescription>{description}</EmptyDescription>
        ) : null}
      </EmptyHeader>
      {action === "none" ? null : (
        <EmptyActions>
          <ActionFor
            action={action}
            onClearSearch={onClearSearch}
            onClearFilters={onClearFilters}
          />
        </EmptyActions>
      )}
    </Empty>
  );
}
