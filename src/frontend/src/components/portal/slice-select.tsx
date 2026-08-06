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
import {
  usePortalNavActions,
  usePortalSlice,
} from "@/lib/portal/portal-nav";

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
  const current = slice || TEAM_KEY;
  const value = all.some((d) => d.key === current) ? current : TEAM_KEY;
  const label = all.find((d) => d.key === value)?.label ?? "Team (all)";
  return (
    <Select
      value={value}
      onValueChange={(v) => setSlice(v && v !== TEAM_KEY ? v : "")}
    >
      <SelectTrigger size="sm" aria-label="Slice by" className="w-32 md:w-48">
        <SelectValue>
          <span className="hidden md:inline">Slice: </span>
          {label}
        </SelectValue>
      </SelectTrigger>
      <SelectContent align="end">
        <SelectGroup>
          <SelectLabel className="text-xs text-muted-foreground">
            Slice by
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
