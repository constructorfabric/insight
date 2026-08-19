import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { SliceDim } from "@/lib/insight/slices";
import { PLANNED_SLICES } from "@/lib/insight/slices";
import { NO_COHORT_REASON } from "@/lib/portal/cohort-options";
import { usePortalNavActions, usePortalSlice } from "@/lib/portal/portal-nav";

/**
 * "No slice" — the whole roster is one cohort and views stay per-person. The
 * store keeps this as `""`; the Select uses a non-empty sentinel because Base
 * UI treats an empty-string value as "no selection" (blank trigger). The
 * sentinel is underscored so a roster attribute literally named `team` cannot
 * collide with it — that collision duplicated a React key and made the real
 * dimension unselectable, because picking it read as "no slice".
 */
const TEAM_KEY = "__team__";
const TEAM_SLICE = { key: TEAM_KEY, label: "Team (all)" };

/**
 * The one shared slice control. Writes the global `portal.slice`, so picking a
 * dimension re-cohorts every view (roster heat, attention, AI cost, …) at once.
 * `dims` are the data-derived slices for the current roster; planned dims are
 * appended (and render ComingSoon in the consuming view).
 */
export function SliceSelect({ dims }: { dims: SliceDim[] }) {
  const { setSlice } = usePortalNavActions();
  const slice = usePortalSlice();
  const all = [TEAM_SLICE, ...dims, ...PLANNED_SLICES];
  // Slices are discovered by enumerating people and grouping them by an
  // attribute, so a viewer whose roster holds only themselves has nothing to
  // group: identity serves a viewer their own subtree, and an individual
  // contributor's subtree is one person. Their attributes are all there —
  // there is simply no second value for any of them.
  //
  // The comparisons on screen still happen: the peer view compares within the
  // person's organization unit, decided server-side. What is missing is the
  // CHOICE, and a control offering exactly one option states the opposite —
  // it reads as a setting the reader picked, not as the only thing available.
  const hasChoice = dims.length + PLANNED_SLICES.length > 0;
  const current = slice || TEAM_KEY;
  const value = all.some((d) => d.key === current) ? current : TEAM_KEY;
  const label = all.find((d) => d.key === value)?.label ?? "Team (all)";
  return (
    <Select
      value={value}
      onValueChange={(v) => setSlice(v && v !== TEAM_KEY ? v : "")}
    >
      {/* The trigger carries the VALUE only; the word "Cohort" labels the
          control from outside, next to the tooltip that explains it. Inside,
          it read as part of the value and repeated on every option list. */}
      <SelectTrigger
        size="sm"
        aria-label="Cohort"
        className="w-32 md:w-44"
        disabled={!hasChoice}
        title={hasChoice ? undefined : NO_COHORT_REASON}
      >
        <SelectValue>{label}</SelectValue>
      </SelectTrigger>
      <SelectContent align="end">
        <SelectGroup>
          <SelectLabel className="text-xs text-muted-foreground">
            Compare against
          </SelectLabel>
          {all.map((d) => (
            <SelectItem key={d.key} value={d.key}>
              {d.label}
            </SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  );
}
