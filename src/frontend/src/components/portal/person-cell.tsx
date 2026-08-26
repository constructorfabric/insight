/**
 * A person as a compact, recognisable cell: avatar initials, the best name
 * the journal knows, and the identifying line under it. Shared by every
 * identity-console surface (queue candidates, detail panel, picker) so the
 * same person always reads the same way.
 *
 * Field precedence mirrors the backend card: `display_name`, else a
 * source-native username, else email — a git-only identity is recognisable
 * by its handle, and a generated address is not what anybody calls them. A `terminated` status is marked so nobody merges INTO a leaver by
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
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
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
          <PersonMarks person={person} />
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

/**
 * What makes a person the wrong one to act on: a stub automation minted, or a
 * leaver. Wherever a person is named for a decision — a cell in a listing, the
 * heading of the window they are decided in — these travel with the name.
 */
export function PersonMarks({ person }: { person: PersonSummary }) {
  const { t } = useTranslation();
  return (
    <>
      {/* One word on the badge, the warning behind it: a badge never wraps and
          never shrinks, so a sentence in one pushes the name out of a narrow
          column and spills across the row. The trigger takes a tab stop of its
          own on purpose — this mark is what says a person is the wrong side of a
          merge, and a hover-only carrier tells a keyboard reader nothing. */}
      {person.provisional ? (
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger
              render={
                <Badge
                  variant="outline"
                  className="cursor-help font-normal"
                  tabIndex={0}
                />
              }
            >
              {t("identities.person.provisional")}
            </TooltipTrigger>
            <TooltipContent side="top">
              {t("identities.person.provisional_hint")}
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      ) : null}
      {person.status?.trim().toLowerCase() === TERMINATED ? (
        <Badge variant="secondary" className="bg-destructive/15 text-destructive">
          {t("identities.person.terminated")}
        </Badge>
      ) : null}
    </>
  );
}
