export interface SubsystemTone {
  /** Bar fill + text, for the schedule. */
  bar: string;
  /** Chip fill + text, for the roadmap grid. */
  chip: string;
  /** Solid dot, for a legend or a filter control. */
  dot: string;
}

const TONES: readonly SubsystemTone[] = [
  {
    bar: "bg-sky-600/85 text-white",
    chip: "bg-sky-500/15 text-sky-700 dark:text-sky-300",
    dot: "bg-sky-600",
  },
  {
    bar: "bg-violet-600/85 text-white",
    chip: "bg-violet-500/15 text-violet-700 dark:text-violet-300",
    dot: "bg-violet-600",
  },
  {
    bar: "bg-teal-600/85 text-white",
    chip: "bg-teal-500/15 text-teal-700 dark:text-teal-300",
    dot: "bg-teal-600",
  },
  {
    bar: "bg-amber-600/85 text-white",
    chip: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
    dot: "bg-amber-600",
  },
  {
    bar: "bg-rose-600/85 text-white",
    chip: "bg-rose-500/15 text-rose-700 dark:text-rose-300",
    dot: "bg-rose-600",
  },
  {
    bar: "bg-indigo-600/85 text-white",
    chip: "bg-indigo-500/15 text-indigo-700 dark:text-indigo-300",
    dot: "bg-indigo-600",
  },
  {
    bar: "bg-emerald-600/85 text-white",
    chip: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
    dot: "bg-emerald-600",
  },
  {
    bar: "bg-fuchsia-600/85 text-white",
    chip: "bg-fuchsia-500/15 text-fuchsia-700 dark:text-fuchsia-300",
    dot: "bg-fuchsia-600",
  },
];

const UNKNOWN: SubsystemTone = {
  bar: "bg-slate-500/80 text-white",
  chip: "bg-slate-500/15 text-slate-700 dark:text-slate-300",
  dot: "bg-slate-500",
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
