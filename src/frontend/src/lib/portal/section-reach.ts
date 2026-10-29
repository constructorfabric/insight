import type { SectionStanding } from "@/lib/portal/use-person-sections";

/** Nothing feeding a section and a person doing none of it stay apart. */
export type SectionReach = "measured" | "quiet" | "unconnected";

export function sectionReach(
  standing: Pick<SectionStanding, "hasData" | "peersHaveData">
): SectionReach {
  if (standing.hasData) return "measured";
  return standing.peersHaveData ? "quiet" : "unconnected";
}

export const SECTION_REACH_LABEL: Record<SectionReach, string> = {
  measured: "Has data",
  quiet: "Nothing recorded this period",
  unconnected: "No data source connected",
};
