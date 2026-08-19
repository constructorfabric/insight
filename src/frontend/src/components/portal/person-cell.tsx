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
 *
 * A person minted at a first sign-in is marked wherever they appear: the
 * picker is where the wrong one gets chosen, and merging INTO such a stub is
 * the wrong direction — the history is on the other side.
 *
 * The `person_id` is always shown and always copyable. A conflict is normally
 * two records of the same human, so name and address are exactly the fields
 * that fail to tell them apart — the id is the one that never does, and it is
 * what an operator pastes into a search, a query or a ticket.
 */
import { useTranslation } from "react-i18next";

import type { PersonSummary } from "@/api/identity-client";
import { CopyValueButton } from "@/components/copy-value-button";
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
  const resolved = personDisplayName(person);
  // Falling back to the id would print it as a name over the id line below.
  // A person the journal holds no attributes for — one minted at first
  // sign-in, before the resolver attached the roster's name — reads as
  // unnamed, and its id right under it is the whole identity it has.
  const named = resolved !== person.person_id;
  const name = named ? resolved : t("identities.person.unnamed");
  // The second line identifies, never repeats: whichever field became the
  // name is skipped here.
  const detail = [person.email, person.username, person.job_title]
    .map((s) => s?.trim())
    .filter((s): s is string => Boolean(s) && s !== name)
    .join(" · ");

  return (
    <div className={cn("flex min-w-0 items-center gap-2", className)}>
      <Avatar className="size-8">
        <AvatarFallback>{named ? getInitials(resolved) : "?"}</AvatarFallback>
      </Avatar>
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <span
            className={cn(
              "truncate text-sm font-medium",
              named || "text-muted-foreground italic",
            )}
          >
            {name}
          </span>
          {person.provisional ? (
            <Badge variant="outline" className="font-normal">
              {t("identities.person.provisional")}
            </Badge>
          ) : null}
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
        <div className="flex items-center gap-1">
          <span className="truncate font-mono text-xs text-muted-foreground select-text">
            {person.person_id}
          </span>
          <CopyValueButton
            value={person.person_id}
            title={t("identities.person.copy_id")}
            copyLabel={t("common.copy")}
            copiedLabel={t("common.copied")}
            errorMessage={t("common.copy_failed")}
          />
        </div>
      </div>
    </div>
  );
}
