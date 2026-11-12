import type { ConnectorTone } from "@/lib/portal/connector-health";

/** Tone carries emphasis; the word beside it carries the meaning. */
export const TONE_STYLE: Record<ConnectorTone, string> = {
  failing: "bg-destructive/15 text-destructive",
  unknown: "bg-warning/15 text-warning",
  active: "bg-primary/15 text-primary",
  ok: "bg-success/15 text-success",
  idle: "bg-muted text-muted-foreground",
};

export const TONE_BAR_STYLE: Record<ConnectorTone, string> = {
  failing: "bg-destructive",
  unknown: "bg-warning",
  active: "bg-primary",
  ok: "bg-success",
  idle: "bg-muted-foreground",
};
