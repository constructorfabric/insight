import { Check, Clock, Minus } from "lucide-react";
import type {
  SchedulePendingReason,
  ScheduleStatus,
} from "@/api/connector-health-client";

export function scheduleStatusView(
  status: ScheduleStatus | null | undefined,
  reason?: SchedulePendingReason | null
) {
  const label =
    status === "pending"
      ? reason === "no_cronworkflow"
        ? "Not scheduled yet"
        : "Waiting for reconcile"
      : status === "applied"
        ? "Applied"
        : status === "inherited"
          ? "Default"
          : status === "paused"
            ? "Paused"
            : "Unavailable";
  const Icon =
    status === "applied" ? Check : status === "pending" ? Clock : Minus;
  const tone =
    status === "applied"
      ? "bg-success/15 text-success"
      : status === "pending"
        ? "bg-warning/15 text-warning"
        : "text-muted-foreground";
  return { label, Icon, tone, iconClass: "size-3 shrink-0" };
}
