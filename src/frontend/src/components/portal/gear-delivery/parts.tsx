import type { AssigneeLink } from "@/api/gear-roadmap-client";
import { RecordLink } from "@/components/record-link";
import { NO_METRIC_VALUE, formatMetricValue } from "@/lib/format";

/**
 * The logins a gear carries, each linked to its account page where a
 * configured source knows one.
 */
export function AssigneeLinks({
  logins,
  links,
}: {
  logins: string[];
  links: AssigneeLink[] | undefined;
}) {
  if (links === undefined || links.length === 0) {
    return <>{logins.join(", ") || NO_METRIC_VALUE}</>;
  }

  return (
    <span className="flex flex-wrap gap-x-2">
      {links.map((assignee) => (
        <RecordLink key={assignee.login} href={assignee.url ?? undefined}>
          {assignee.login}
        </RecordLink>
      ))}
    </span>
  );
}

/** A share as a bar plus its number; a dash where nothing carries a value. */
export function ShareBar({
  value,
  width,
}: {
  value: number | null | undefined;
  width: string;
}) {
  if (value == null || !Number.isFinite(value)) {
    return <span className="text-muted-foreground">{NO_METRIC_VALUE}</span>;
  }

  return (
    <span className="flex items-center gap-2">
      <span
        className={`h-1.5 ${width} overflow-hidden rounded-full bg-muted`}
      >
        <span
          className={`block h-full rounded-full ${
            value >= 100 ? "bg-emerald-600/70" : "bg-primary/70"
          }`}
          style={{ width: `${Math.min(Math.max(value, 0), 100)}%` }}
        />
      </span>
      <span className="text-xs tabular-nums text-muted-foreground">
        {formatMetricValue(value, "percent")}
      </span>
    </span>
  );
}
