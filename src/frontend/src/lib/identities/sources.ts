export function normalizeSources(sources: readonly string[]): string[] {
  const cleaned = sources
    .map((source) => source.trim())
    .filter((source) => source !== "");
  return [...new Set(cleaned)].sort();
}

export function parseSources(src: string | undefined): string[] {
  return src ? normalizeSources(src.split(",")) : [];
}

export function formatSources(sources: readonly string[]): string | undefined {
  const normalized = normalizeSources(sources);
  return normalized.length > 0 ? normalized.join(",") : undefined;
}
