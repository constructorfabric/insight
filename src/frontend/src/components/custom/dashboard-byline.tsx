import { Link } from "@tanstack/react-router";

import type {
  DashboardAccess,
  DashboardVisibility,
  Folder,
} from "@/api/custom-client";
import { Badge } from "@/components/ui/badge";
import { dashboardAllows } from "@/lib/custom/dashboard-access";
import { changedAgo } from "@/lib/custom/changed-ago";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const VISIBILITY: Record<
  DashboardVisibility,
  { label: string; variant: "secondary" | "info" }
> = {
  private: { label: "Private", variant: "secondary" },
  people: { label: "Shared with people", variant: "info" },
  everyone: { label: "Shared with org", variant: "info" },
};

/** Owner · last change · where it is filed · who else sees it, as a card and the open dashboard show them. */
export function DashboardByline({
  owner,
  updatedAt,
  visibility,
  access,
  folders = [],
  className,
}: {
  owner: string | null | undefined;
  updatedAt: string | undefined;
  visibility: DashboardVisibility | null | undefined;
  access: DashboardAccess;
  /** The folders holding it that the viewer sees. */
  folders?: readonly Folder[];
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
      {folders.length > 0 ? <FiledIn folders={folders} /> : null}
      {shareable && shown ? (
        <Badge variant={shown.variant} className="shrink-0">
          {shown.label}
        </Badge>
      ) : null}
    </div>
  );
}

function FiledIn({ folders }: { folders: readonly Folder[] }) {
  const [first, ...rest] = folders;

  return (
    <span className="min-w-0 truncate not-first:before:me-1.5 not-first:before:content-['·']">
      in{" "}
      <Link
        to="/portal/custom"
        search={{ folder: first.id }}
        className="underline-offset-4 hover:text-foreground hover:underline"
      >
        {first.name}
      </Link>
      {rest.length > 0 ? (
        <span title={rest.map((folder) => folder.name).join(", ")}>
          {" "}
          +{rest.length}
        </span>
      ) : null}
    </span>
  );
}
