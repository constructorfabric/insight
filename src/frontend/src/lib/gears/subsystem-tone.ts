export interface SubsystemTone {
  /** Bar fill + text, for the schedule. */
  bar: string;
  /** Chip fill + text, for the roadmap grid. */
  chip: string;
}

const TONES: readonly SubsystemTone[] = [
  { bar: "bg-sky-600/85 text-white", chip: "bg-sky-500/12 text-sky-800 dark:text-sky-200" },
  { bar: "bg-violet-600/85 text-white", chip: "bg-violet-500/12 text-violet-800 dark:text-violet-200" },
  { bar: "bg-teal-600/85 text-white", chip: "bg-teal-500/12 text-teal-800 dark:text-teal-200" },
  { bar: "bg-amber-600/85 text-white", chip: "bg-amber-500/12 text-amber-800 dark:text-amber-200" },
  { bar: "bg-rose-600/85 text-white", chip: "bg-rose-500/12 text-rose-800 dark:text-rose-200" },
  { bar: "bg-indigo-600/85 text-white", chip: "bg-indigo-500/12 text-indigo-800 dark:text-indigo-200" },
  { bar: "bg-emerald-600/85 text-white", chip: "bg-emerald-500/12 text-emerald-800 dark:text-emerald-200" },
  { bar: "bg-fuchsia-600/85 text-white", chip: "bg-fuchsia-500/12 text-fuchsia-800 dark:text-fuchsia-200" },
];

const UNKNOWN: SubsystemTone = {
  bar: "bg-slate-500/80 text-white",
  chip: "bg-slate-500/12 text-slate-700 dark:text-slate-200",
};

/**
 * A stable colour per subsystem, keyed by name rather than by position, so the
 * same subsystem keeps its colour as the board grows.
 */
export function subsystemTone(subsystem: string | null): SubsystemTone {
  if (subsystem === null || subsystem === "") return UNKNOWN;

  let hash = 0;
  for (const character of subsystem) {
    hash = (hash * 31 + character.codePointAt(0)!) % 1_000_003;
  }

  return TONES[hash % TONES.length]!;
}
