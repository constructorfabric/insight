import { Link } from "@tanstack/react-router";

import { Card, CardContent } from "@/components/ui/card";
import { useSettings } from "@/hooks/use-settings";
import { normalizePersonId } from "@/lib/metrics/entity";
import { worstEntry, type PeerStoryEntry } from "@/lib/metrics/peer-story";
import { applyFocus, PEER_TEXT } from "@/lib/peers";
import { cn } from "@/lib/utils";
import type { TeamMember } from "@/types/insight";

export interface TeamMembersAttentionProps {
  members: TeamMember[];
  /**
   * Per-member below-peer counts across all group collections, keyed by
   * normalized person id. Each member is counted against THEIR OWN department
   * cohort (resolved by the peer view).
   */
  metricBelowByMember: Map<string, number>;
  /**
   * Per-person peer-story entries across all groups, keyed by normalized person
   * id — the source of each member's "worst" headline metric.
   */
  metricEntriesByPerson: Map<string, PeerStoryEntry[]>;
}

export function TeamMembersAttention({
  members,
  metricBelowByMember,
  metricEntriesByPerson,
}: TeamMembersAttentionProps) {
  const { focusMode } = useSettings();

  const attention = members
    .map((m) => {
      const entityId = normalizePersonId(m.person_id);
      const belowCount = metricBelowByMember.get(entityId) ?? 0;
      const worstLabel =
        worstEntry(metricEntriesByPerson.get(entityId) ?? [])?.label ?? null;
      return { member: m, belowCount, worstLabel };
    })
    .filter((x) => x.belowCount > 0)
    .sort((a, b) => b.belowCount - a.belowCount)
    .slice(0, 6);

  if (attention.length === 0) return null;

  const subtitle =
    members.length > 0
      ? `${members.length} members · vs department peers`
      : "vs department peers";
  const badStatus = applyFocus("bottom", focusMode);

  return (
    <section>
      <h2 className="mb-3 text-xs font-medium tracking-wider text-muted-foreground uppercase">
        Members needing attention
      </h2>
      <Card data-size="sm">
        <CardContent className="flex flex-col gap-2 text-sm">
          <span className="text-[11px] text-muted-foreground">{subtitle}</span>
          <ul className="grid grid-cols-1 gap-x-8 gap-y-1 md:grid-cols-2">
            {attention.map(({ member, belowCount, worstLabel }) => (
              <li key={member.person_id}>
                <Link
                  to="/ic/$person/personal"
                  params={{ person: member.person_id }}
                  className="-mx-2 flex w-[calc(100%+1rem)] items-baseline gap-2 rounded px-2 py-1 text-left text-sm no-underline! transition-colors hover:bg-accent"
                >
                  <span className="min-w-0 truncate text-foreground">
                    {member.name}
                  </span>
                  {worstLabel ? (
                    <span className="min-w-0 truncate text-xs text-muted-foreground">
                      worst: {worstLabel}
                    </span>
                  ) : null}
                  <span
                    className={cn(
                      "ml-auto shrink-0 font-mono font-bold tabular-nums",
                      PEER_TEXT[badStatus],
                    )}
                  >
                    {belowCount}
                  </span>
                  <span className="shrink-0 whitespace-nowrap text-muted-foreground">
                    trailing
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>
    </section>
  );
}
