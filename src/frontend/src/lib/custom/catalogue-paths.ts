/** The kinds that have a page of their own to look at, and where it is. */
export const PREVIEWED = {
  datasets: "/dashboards/datasets/$name",
  metrics: "/dashboards/metrics/$name",
  widgets: "/dashboards/widgets/$name",
} as const;

export type PreviewedKind = keyof typeof PREVIEWED;
