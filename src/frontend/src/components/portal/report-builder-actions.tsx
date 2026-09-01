import { X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

interface ReportBuilderActionsProps {
  selectedCount: number;
  hasSelection: boolean;
  blocker: string | null;
  running: boolean;
  failure: string | null;
  onClear: () => void;
  onCancel: () => void;
  onPreview: () => void;
}

export function ReportBuilderActions({
  selectedCount,
  hasSelection,
  blocker,
  running,
  failure,
  onClear,
  onCancel,
  onPreview,
}: ReportBuilderActionsProps) {
  return (
    <div className="fixed end-0 bottom-0 start-0 z-30 border-t bg-background/95 backdrop-blur-sm md:start-[var(--rail-width)] lg:start-[calc(var(--rail-width)+var(--sidebar-width))]">
      <div className="mx-auto flex max-w-screen-2xl flex-wrap items-center gap-3 px-4 py-3 md:px-6">
        <div className="flex min-w-0 flex-1 flex-col">
          <span
            role="status"
            aria-label={`${selectedCount} selected`}
            className="text-sm text-muted-foreground"
          >
            <strong className="font-semibold text-foreground">
              {selectedCount}
            </strong>{" "}
            selected
          </span>
          {failure ? (
            <span className="truncate text-sm text-destructive">{failure}</span>
          ) : blocker && hasSelection && !running ? (
            <span className="truncate text-xs text-muted-foreground">
              {blocker}
            </span>
          ) : running ? (
            <span className="flex items-center gap-2 text-xs text-muted-foreground">
              <Spinner className="size-3" /> Preparing report…
            </span>
          ) : null}
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={!hasSelection || running}
          onClick={onClear}
        >
          Clear
        </Button>
        {running ? (
          <Button type="button" variant="outline" onClick={onCancel}>
            <X className="size-4" /> Cancel
          </Button>
        ) : null}
        <Button
          type="button"
          disabled={blocker != null || running}
          onClick={onPreview}
        >
          Preview report
        </Button>
      </div>
    </div>
  );
}
