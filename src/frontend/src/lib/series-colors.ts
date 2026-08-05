/**
 * Series colors for dimension-split charts.
 *
 * Known tools and sources get a stable identity color — a vendor-brand hue
 * tuned per ground via the `--brand-*` theme tokens — so the same tool reads
 * the same on every chart, drilldown, and theme. The map covers every
 * connector in the platform's documented inventory; families that are not
 * built yet use their anticipated dimension value (snake_case vendor name)
 * and simply fall back to chart tokens until the value matches. Unknown
 * seeds fall back to the theme's categorical chart tokens (`--chart-1..12`),
 * assigned in sorted seed order so a given seed set always maps the same
 * way; past twelve they cycle.
 *
 * A `Map` (not a `Record`) so a malformed seed like `"__proto__"` or
 * `"constructor"` falls through to the chart-token path instead of leaking
 * `Object.prototype` members as colors.
 */
const BRAND_BY_SEED: ReadonlyMap<string, string> = new Map([
  // collaboration
  ["slack", "var(--brand-slack)"],
  ["m365", "var(--brand-m365)"],
  ["zoom", "var(--brand-zoom)"],
  ["zulip", "var(--brand-zulip)"],
  ["zulip_proxy", "var(--brand-zulip)"],
  // git
  ["github", "var(--brand-github)"],
  ["gitlab", "var(--brand-gitlab)"],
  ["bitbucket_cloud", "var(--brand-bitbucket)"],
  ["bitbucket_server", "var(--brand-bitbucket-server)"],
  // ai
  ["cursor", "var(--brand-cursor)"],
  ["claude", "var(--brand-claude)"],
  ["claude_code", "var(--brand-claude-code)"],
  ["chatgpt", "var(--brand-chatgpt)"],
  ["codex", "var(--brand-codex)"],
  ["openai", "var(--brand-openai)"],
  ["copilot", "var(--brand-copilot)"],
  ["jetbrains", "var(--brand-jetbrains)"],
  ["windsurf", "var(--brand-windsurf)"],
  // task tracking
  ["jira", "var(--brand-jira)"],
  ["youtrack", "var(--brand-youtrack)"],
  // crm
  ["hubspot", "var(--brand-hubspot)"],
  ["salesforce", "var(--brand-salesforce)"],
  // support
  ["jsm", "var(--brand-jsm)"],
  ["zendesk", "var(--brand-zendesk)"],
  // wiki
  ["confluence", "var(--brand-confluence)"],
  ["outline", "var(--brand-outline)"],
  // testing
  ["allure", "var(--brand-allure)"],
  // ui design
  ["figma", "var(--brand-figma)"],
  // hr directory
  ["bamboohr", "var(--brand-bamboohr)"],
  ["workday", "var(--brand-workday)"],
  ["ms_entra", "var(--brand-ms-entra)"],
  ["ldap", "var(--brand-ldap)"],
]);

const CHART_TOKENS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
  "var(--chart-6)",
  "var(--chart-7)",
  "var(--chart-8)",
  "var(--chart-9)",
  "var(--chart-10)",
  "var(--chart-11)",
  "var(--chart-12)",
] as const;

export function seriesColors(seeds: string[]): Record<string, string> {
  // Null prototype so a seed like "__proto__" becomes an own property
  // instead of silently mutating (or dropping) the object's prototype.
  const palette: Record<string, string> = Object.create(null) as Record<
    string,
    string
  >;
  const unknown: string[] = [];
  for (const seed of new Set(seeds)) {
    const brand = BRAND_BY_SEED.get(seed);
    if (brand) palette[seed] = brand;
    else unknown.push(seed);
  }
  unknown.sort();
  unknown.forEach((seed, index) => {
    palette[seed] = CHART_TOKENS[index % CHART_TOKENS.length]!;
  });
  return palette;
}
