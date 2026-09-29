const TAG_COLLATOR = new Intl.Collator("en", { sensitivity: "accent" });

export function sameTagName(a: string, b: string): boolean {
  return TAG_COLLATOR.compare(a, b) === 0;
}
