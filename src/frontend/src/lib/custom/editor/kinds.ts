import type { EditableKind } from "@/api/custom-client";

export const NOUNS: Readonly<Record<EditableKind, string>> = {
  datasets: "dataset",
  metrics: "metric",
  widgets: "widget",
  dashboards: "dashboard",
};
