import { Database, Ellipsis } from "lucide-react";

import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";
import {
  useEvidenceScope,
  useMetricEvidenceOptional,
  withOwnTarget,
} from "@/components/metric-evidence-context";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export function MetricCardActions({
  evidence,
  label,
  placement = "overlay",
}: {
  evidence: MetricEvidenceSelection | null | undefined;
  label: string;
  // `overlay` takes the button out of flow, so the call site owes it clear space.
  placement?: "overlay" | "inline";
}) {
  const evidenceContext = useMetricEvidenceOptional();
  const scope = useEvidenceScope();
  if (!evidence || !evidenceContext) return null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className={cn(
              "text-muted-foreground",
              placement === "overlay" && "absolute top-4 right-4 z-10"
            )}
            aria-label={`More actions for ${label}`}
            onClick={(event) => event.stopPropagation()}
            icon={<Ellipsis />}
          />
        }
      />
      <DropdownMenuContent align="end" className="w-48">
        <DropdownMenuItem
          onClick={(event) => {
            event.stopPropagation();
            evidenceContext.openEvidenceTargets(
              withOwnTarget(scope, { selection: evidence, label }),
              { activeMetricKey: evidence.metric_key }
            );
          }}
        >
          <Database />
          View supporting data
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
