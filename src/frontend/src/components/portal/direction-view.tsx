import { useMemo } from "react";

import { DomainLensView, Pending } from "@/components/portal/domain-lens-view";
import { directionMetricKeys, lensEntry } from "@/lib/portal/lens-configs";
import { DIRECTIONS } from "@/lib/portal/nav-model";

/**
 * Directions content — every direction/lens resolves through the lens-config
 * registry: a section config renders DomainLensView; a roadmap entry renders
 * its honest ComingSoon note (design D1). The grid fetch is direction-scoped
 * (union of the direction's lens keys) so switching lenses never re-spins.
 */
export function DirectionView({ dir, lens }: { dir: string; lens: string }) {
  const entry = lensEntry(dir, lens);
  const direction = DIRECTIONS.find((d) => d.id === dir);
  const gridKeys = useMemo(() => directionMetricKeys(dir), [dir]);
  // Collapsing a direction in the pane clears `dir`, so there is no direction
  // to talk about — say "pick one" instead of blaming the lens for not
  // existing in a direction the reader never named.
  if (!direction) {
    return <Pending label="Pick a direction to see its metrics." />;
  }
  if (!entry) {
    return (
      <Pending label={`“${lens}” isn't a metric family in ${direction.name} yet.`} />
    );
  }
  if ("comingSoon" in entry) return <Pending label={entry.comingSoon} />;
  return <DomainLensView config={entry} gridKeys={gridKeys} />;
}
