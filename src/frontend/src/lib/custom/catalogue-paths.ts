/** The kinds that have a page of their own to look at, and where it is. */
export const PREVIEWED = {
  datasets: "/portal/custom/datasets/$name",
  metrics: "/portal/custom/metrics/$name",
  widgets: "/portal/custom/widgets/$name",
} as const;

export type PreviewedKind = keyof typeof PREVIEWED;
