import { pathId } from "@/lib/alerts/condition";

export function described(ids: readonly string[]) {
  return ids.length === 0
    ? {}
    : { "aria-describedby": ids.join(" "), "aria-invalid": true as const };
}

export function controlId(path: string): string {
  return `alert-${pathId(`condition.${path}`)}`;
}
