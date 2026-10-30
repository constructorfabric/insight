import type { DashboardAccess, DashboardVisibility } from "@/api/custom-client";
import { Badge } from "@/components/ui/badge";
import { dashboardAllows } from "@/lib/custom/dashboard-access";
import { describeAge } from "@/lib/portal/connector-health";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const DAY_MS = 24 * 60 * 60 * 1000;

const VISIBILITY: Record<
  DashboardVisibility,
  { label: string; variant: "secondary" | "info" }
> = {
  private: { label: "Private", variant: "secondary" },
  people: { label: "Shared with people", variant: "info" },
  everyone: { label: "Shared with org", variant: "info" },
};

function changedAgo(updatedAt: string): string | null {
  const changed = Date.parse(updatedAt);
  if (Number.isNaN(changed)) return null;

  const age = Math.max(0, Date.now() - changed);
  if (age >= DAY_MS && age < 2 * DAY_MS) return "yesterday";

  return describeAge(age);
}

/** Owner · last change · who else sees it, as a card and the open dashboard show them. */
export function DashboardByline({
  owner,
  updatedAt,
  visibility,
  access,
  className,
}: {
  owner: string | null | undefined;
  updatedAt: string | undefined;
  visibility: DashboardVisibility | null | undefined;
  access: DashboardAccess;
  className?: string;
}) {
  const changed = updatedAt ? changedAgo(updatedAt) : null;
  const shown = visibility ? VISIBILITY[visibility] : undefined;
  const shareable = dashboardAllows(access).share;

  return (
    <div
      className={cn(
        TEXT_LABEL,
        "flex min-h-5 min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 font-normal",
        className
      )}
    >
      {owner ? <span className="max-w-full truncate">{owner}</span> : null}
      {changed ? (
        <time
          dateTime={updatedAt}
          className="shrink-0 whitespace-nowrap not-first:before:me-1.5 not-first:before:content-['·']"
        >
          {changed}
        </time>
      ) : null}
      {shareable && shown ? (
        <Badge variant={shown.variant} className="shrink-0">
          {shown.label}
        </Badge>
      ) : null}
    </div>
  );
}
