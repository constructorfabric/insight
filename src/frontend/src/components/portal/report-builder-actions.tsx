import { X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button, Spinner } from "@gears-frontx/ui-kit";

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
  const { t } = useTranslation();
  return (
    <div className="fixed end-0 bottom-0 start-0 z-30 border-t bg-background/95 backdrop-blur-sm md:start-[var(--rail-width)] lg:start-[calc(var(--rail-width)+var(--sidebar-width))]">
      <div className="mx-auto flex max-w-screen-2xl flex-wrap items-center gap-3 px-4 py-3 md:px-6">
        <div className="flex min-w-0 flex-1 flex-col">
          <span
            role="status"
            aria-label={t("reports.selected", { count: selectedCount })}
            className="text-sm text-muted-foreground"
          >
            <strong className="font-semibold text-foreground">
              {selectedCount}
            </strong>{" "}
            {t("reports.selected_word")}
          </span>
          {failure ? (
            <span className="truncate text-sm text-destructive">{failure}</span>
          ) : blocker && hasSelection && !running ? (
            <span className="truncate text-xs text-muted-foreground">
              {blocker}
            </span>
          ) : running ? (
            <span className="flex items-center gap-2 text-xs text-muted-foreground">
              <Spinner className="size-3" /> {t("reports.preparing")}
            </span>
          ) : null}
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={!hasSelection || running}
          onClick={onClear}
        >
          {t("common.actions.clear")}
        </Button>
        {running ? (
          <Button type="button" variant="outline" onClick={onCancel} icon={<X className="size-4" />}>
            {t("common.actions.cancel")}
          </Button>
        ) : null}
        <Button
          type="button"
          disabled={blocker != null || running}
          onClick={onPreview}
        >
          {t("reports.preview")}
        </Button>
      </div>
    </div>
  );
}
