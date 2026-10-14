import type { AlertPreview } from "@/api/alerts-types";
import { refusal } from "@/components/custom/refusal";
import { Spinner } from "@/components/ui/spinner";
import { reasonText } from "@/lib/alerts/describe";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { savingRefusal } from "./field-refusals";
import type { AlertPreviewState } from "./use-alert-preview";
import { WitnessTable } from "./witness";

function outcomeText(preview: AlertPreview, ratioOnly: boolean): string {
  switch (preview.outcome) {
    case "breach":
      return "Now: meets the condition";
    case "no_breach":
      return "Now: does not meet the condition";
    case "unknown": {
      const reason =
        preview.reason === null
          ? "Unknown"
          : ratioOnly && preview.reason === "null"
            ? "No data in this window"
            : reasonText(preview.reason);
      return `Now: unknown · ${reason}`;
    }
  }
}

function refusedText(error: unknown): string {
  const { fields } = savingRefusal(error);
  return Object.keys(fields).length > 0
    ? "Fix the highlighted fields to check."
    : refusal(error, "Couldn't check the condition.");
}

export function CurrentValue({
  preview,
  ratioOnly,
}: {
  preview: AlertPreviewState;
  ratioOnly: boolean;
}) {
  const { ready, settled, query } = preview;
  const found = ready && settled && query.isSuccess ? query.data : undefined;

  return (
    <div className={cn(TEXT_LABEL, "flex flex-col gap-2")}>
      <p
        aria-live="polite"
        aria-atomic="true"
        className={cn(
          (query.isError || found?.outcome === "unknown") && "text-destructive"
        )}
      >
        {line()}
      </p>
      {found?.witness ? <WitnessTable witness={found.witness} /> : null}
    </div>
  );

  function line() {
    if (!ready) return null;
    if (!settled || query.isPending) {
      return (
        <span className="flex items-center gap-2">
          <Spinner className="size-3" /> Checking…
        </span>
      );
    }
    if (query.isError) return refusedText(query.error);

    return outcomeText(query.data, ratioOnly);
  }
}
