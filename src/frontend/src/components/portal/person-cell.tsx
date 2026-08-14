/**
 * A person as a compact, recognisable cell: avatar initials, the best name
 * the journal knows, and the identifying line under it. Shared by every
 * identity-console surface (queue candidates, detail panel, picker) so the
 * same person always reads the same way.
 *
 * Field precedence mirrors the backend card: `display_name`, else email, else
 * a source-native username — a git-only identity is recognisable by its
 * handle. A `terminated` status is marked so nobody merges INTO a leaver by
 * accident.
 */
import { useTranslation } from "react-i18next";

import type { PersonSummary } from "@/api/identity-client";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { personDisplayName } from "@/lib/identities/person-display";
import { getInitials } from "@/lib/insight/get-initials";
import { cn } from "@/lib/utils";

const TERMINATED = "terminated";

export function PersonCell({
  person,
  className,
}: {
  person: PersonSummary;
  className?: string;
}) {
  const { t } = useTranslation();
  const name = personDisplayName(person);
  // The second line identifies, never repeats: whichever field became the
  // name is skipped here.
  const detail = [person.email, person.username, person.job_title]
    .map((s) => s?.trim())
    .filter((s): s is string => Boolean(s) && s !== name)
    .join(" · ");

  return (
    <div className={cn("flex min-w-0 items-center gap-2", className)}>
      <Avatar className="size-8">
        <AvatarFallback>{getInitials(name)}</AvatarFallback>
      </Avatar>
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm font-medium">{name}</span>
          {person.status?.trim().toLowerCase() === TERMINATED ? (
            <Badge
              variant="secondary"
              className="bg-destructive/15 text-destructive"
            >
              {t("identities.person.terminated")}
            </Badge>
          ) : null}
        </div>
        {detail ? (
          <div className="truncate text-xs text-muted-foreground">{detail}</div>
        ) : null}
      </div>
    </div>
  );
}
