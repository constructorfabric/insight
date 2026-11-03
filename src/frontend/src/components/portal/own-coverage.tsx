import { ChevronRight } from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import type { GroupId } from "@/lib/insight/groups";
import { SECTION_REACH_LABEL, sectionReach } from "@/lib/portal/section-reach";
import { usePersonSectionStandings } from "@/lib/portal/use-person-sections";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function OwnCoverage({
  personId,
  onOpenSection,
}: {
  personId: string;
  onOpenSection: (id: GroupId) => void;
}) {
  const standings = usePersonSectionStandings(personId);

  if (standings.some((standing) => standing.isPending)) {
    return <CenteredSpinner className="min-h-40" />;
  }
  if (standings.some((standing) => standing.isError)) {
    return (
      <ComingSoon
        variant="card"
        state="error"
        onRetry={() => standings.forEach((standing) => standing.refetch())}
      />
    );
  }

  return (
    <Card data-size="sm">
      <CardContent className="p-0">
        <ul aria-label="Sections" className="divide-y">
          {standings.map((standing) => {
            const reach = sectionReach(standing);
            const label = SECTION_REACH_LABEL[reach];

            return (
              <li key={standing.id}>
                <button
                  type="button"
                  aria-label={`${standing.title}: ${label}`}
                  onClick={() => onOpenSection(standing.id)}
                  className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-card-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-inset"
                >
                  <span className={cn(TEXT_BODY, "min-w-0 flex-1 font-medium")}>
                    {standing.title}
                  </span>
                  <span
                    className={cn(
                      TEXT_BODY,
                      reach === "measured"
                        ? "text-foreground"
                        : "text-muted-foreground"
                    )}
                  >
                    {label}
                  </span>
                  <ChevronRight
                    aria-hidden
                    className="size-4 shrink-0 text-muted-foreground"
                  />
                </button>
              </li>
            );
          })}
        </ul>
      </CardContent>
    </Card>
  );
}
