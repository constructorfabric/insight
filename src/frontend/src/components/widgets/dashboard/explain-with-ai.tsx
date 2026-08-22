import { Sparkles } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useAiAvailable, useExplainMetric } from "@/queries/ai";
import type { MetricSnapshot } from "@/api/ai-client";
import { TEXT_HEADING, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export interface ExplainWithAiProps {
  snapshot: MetricSnapshot;
}

/**
 * The sparkle in a tile's corner, and the answer it opens.
 *
 * Renders nothing at all until the deployment offers explanations AND this
 * person has stored a key: an affordance that only reports its own
 * unavailability costs the row's scarcest space and gives nothing back.
 *
 * INVARIANT: mounted as a SIBLING of the tile card, never inside it — the card
 * itself renders as a `<button>`, and a button inside a button is invalid.
 */
export function ExplainWithAi({ snapshot }: ExplainWithAiProps) {
  const { featureOn, hasKey } = useAiAvailable();
  const [open, setOpen] = useState(false);
  const explain = useExplainMetric();

  if (!featureOn || !hasKey) return null;

  const ask = () => {
    setOpen(true);
    explain.mutate(snapshot);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <button
            type="button"
            aria-label={`Explain ${snapshot.label} with AI`}
            onClick={ask}
            className={cn(
              "absolute top-2 right-2 z-10 grid size-7 place-items-center rounded-md",
              "text-muted-foreground transition-colors",
              "hover:bg-accent hover:text-foreground",
              "focus-visible:ring-ring focus-visible:ring-2 focus-visible:outline-none",
              open && "bg-accent text-foreground"
            )}
          >
            <Sparkles className="size-3.5" aria-hidden />
          </button>
        }
      />
      <PopoverContent align="end" className="flex w-96 max-w-[90vw] flex-col gap-3">
        <p className={TEXT_HEADING}>{snapshot.label}, explained</p>
        <ExplanationBody
          isPending={explain.isPending}
          isError={explain.isError}
          text={explain.data?.text ?? ""}
        />
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className={TEXT_LABEL}>
            {explain.data
              ? `${explain.data.model} · ${explain.data.tenant_context_entries} org + ${explain.data.person_context_entries} personal notes`
              : ""}
          </span>
          <Button
            size="sm"
            variant="outline"
            onClick={() => explain.mutate(snapshot)}
            disabled={explain.isPending}
          >
            Try again
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function ExplanationBody({
  isPending,
  isError,
  text,
}: {
  isPending: boolean;
  isError: boolean;
  text: string;
}) {
  if (isPending) {
    return (
      <div className="flex flex-col gap-2" aria-busy>
        <span className="bg-muted h-2.5 w-full animate-pulse rounded-full" />
        <span className="bg-muted h-2.5 w-11/12 animate-pulse rounded-full" />
        <span className="bg-muted h-2.5 w-2/3 animate-pulse rounded-full" />
      </div>
    );
  }

  if (isError) {
    return (
      <p className="text-sm">
        That did not work. Your key may have been rejected — check it in AI
        assistant settings, or try again in a moment.
      </p>
    );
  }

  return <p className="text-sm leading-relaxed whitespace-pre-line">{text}</p>;
}
