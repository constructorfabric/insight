/**
 * The modes the identities console offers. A mode is a way IN to the same
 * decisions — the queue arrives at them from a problem, the person view from a
 * name — so adding one is an entry here and a component, nothing else.
 */
export const MODES = ["queue", "person", "accounts"] as const;

export const DEFAULT_MODE = MODES[0];

/** What earlier releases put in the URL. A link somebody already sent must open
 *  the screen it was sent from, not fall through to the default. */
const RETIRED_MODES: Readonly<Record<string, (typeof MODES)[number]>> = {
  people: "person",
};

export function resolveMode(mode: string | undefined): string {
  if (mode === undefined) return DEFAULT_MODE;
  return MODES.find((m) => m === mode) ?? RETIRED_MODES[mode] ?? DEFAULT_MODE;
}

export const MODE_LABELS: Readonly<Record<string, string>> = {
  queue: "Review queue",
  person: "A person and their accounts",
  accounts: "An account and whose it is",
};
