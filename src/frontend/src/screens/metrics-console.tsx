import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import type { CustomMetricSummary } from "@/api/metrics-client";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { MetricDetail } from "@/components/widgets/metrics-console/metric-detail";
import { MetricEditorDialog } from "@/components/widgets/metrics-console/metric-editor-dialog";
import {
  draftToGraph,
  graphToDraft,
  type MetricDraft,
} from "@/lib/metrics-console/metric-graph";
import {
  useCreateCustomMetric,
  useCustomMetric,
  useCustomMetrics,
  useDeleteCustomMetric,
  useUpdateCustomMetric,
} from "@/queries/custom-metrics";
import { Gauge, Plus, TriangleAlert } from "lucide-react";

type DialogState =
  | { kind: "closed" }
  | { kind: "create" }
  | { kind: "edit"; metricKey: string };

export function MetricsConsoleScreen() {
  const { t } = useTranslation();
  const { data: metrics, isPending, isError } = useCustomMetrics();
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogState>({ kind: "closed" });

  const createMetric = useCreateCustomMetric();
  const deleteMetric = useDeleteCustomMetric();

  const closeDialog = () => setDialog({ kind: "closed" });

  const handleDelete = (metricKey: string) => {
    deleteMetric.mutate(metricKey, {
      onSuccess: () => {
        if (selectedKey === metricKey) setSelectedKey(null);
      },
    });
  };

  return (
    <>
      <header className="sticky top-0 z-20 flex flex-wrap items-center gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur-sm">
        <SidebarTrigger />
        <h1 className="text-xl font-semibold tracking-tight">
          {t("metrics_console.title")}
        </h1>
        <Button
          className="ms-auto"
          size="sm"
          onClick={() => setDialog({ kind: "create" })}
        >
          <Plus />
          {t("metrics_console.new_metric")}
        </Button>
      </header>

      <main className="flex flex-1 flex-col p-4 md:p-6">
        {isPending ? <CenteredSpinner className="min-h-[70vh]" /> : null}

        {isError ? (
          <Alert variant="destructive">
            <TriangleAlert />
            <AlertTitle>{t("metrics_console.list_error")}</AlertTitle>
            <AlertDescription>
              {t("metrics_console.list_error_description")}
            </AlertDescription>
          </Alert>
        ) : null}

        {deleteMetric.isError ? (
          <Alert variant="destructive" className="mb-4">
            <TriangleAlert />
            <AlertTitle>{t("metrics_console.delete_error")}</AlertTitle>
          </Alert>
        ) : null}

        {!isPending && !isError ? (
          <div className="mx-auto grid w-full max-w-6xl flex-1 grid-cols-1 gap-6 md:grid-cols-[18rem_1fr]">
            <MetricList
              metrics={metrics ?? []}
              selectedKey={selectedKey}
              onSelect={setSelectedKey}
            />
            <section className="min-w-0">
              {selectedKey ? (
                <MetricDetail
                  key={selectedKey}
                  metricKey={selectedKey}
                  onEdit={(metricKey) => setDialog({ kind: "edit", metricKey })}
                  onDelete={handleDelete}
                />
              ) : (
                <Empty className="min-h-[50vh]">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <Gauge />
                    </EmptyMedia>
                    <EmptyTitle>{t("metrics_console.no_selection")}</EmptyTitle>
                    <EmptyDescription>
                      {t("metrics_console.no_selection_description")}
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              )}
            </section>
          </div>
        ) : null}
      </main>

      {dialog.kind === "create" ? (
        <MetricEditorDialog
          open
          onOpenChange={(open) => (open ? null : closeDialog())}
          mode="create"
          isPending={createMetric.isPending}
          error={createMetric.error}
          onSubmit={(draft: MetricDraft) =>
            createMetric.mutate(draftToGraph(draft), {
              onSuccess: (created) => {
                setSelectedKey(created.metric_key);
                closeDialog();
              },
            })
          }
        />
      ) : null}

      {dialog.kind === "edit" ? (
        <EditMetricDialog metricKey={dialog.metricKey} onClose={closeDialog} />
      ) : null}
    </>
  );
}

function MetricList({
  metrics,
  selectedKey,
  onSelect,
}: {
  metrics: CustomMetricSummary[];
  selectedKey: string | null;
  onSelect: (metricKey: string) => void;
}) {
  const { t } = useTranslation();

  if (metrics.length === 0) {
    return (
      <aside className="text-sm text-muted-foreground">
        {t("metrics_console.empty_list")}
      </aside>
    );
  }

  return (
    <aside className="flex flex-col gap-1">
      {metrics.map((metric) => (
        <button
          key={metric.metric_key}
          type="button"
          onClick={() => onSelect(metric.metric_key)}
          className={`rounded-md border px-3 py-2 text-start text-sm transition-colors ${
            metric.metric_key === selectedKey
              ? "border-ring bg-muted"
              : "border-transparent hover:bg-muted/60"
          }`}
        >
          <div className="truncate font-medium">{metric.label}</div>
          <div className="mt-1 flex items-center gap-1.5">
            <span className="truncate font-mono text-xs text-muted-foreground">
              {metric.metric_key}
            </span>
            <Badge variant="outline" className="shrink-0">
              {metric.computation}
            </Badge>
          </div>
        </button>
      ))}
    </aside>
  );
}

/**
 * Edit fetches the full metric (list summaries carry no graph) so the form
 * prefills; kept a child so the fetch hook mounts only while editing.
 */
function EditMetricDialog({
  metricKey,
  onClose,
}: {
  metricKey: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const { data: metric, isError } = useCustomMetric(metricKey);
  const updateMetric = useUpdateCustomMetric(metricKey);

  if (isError) {
    return (
      <StatusDialog onClose={onClose}>
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>{t("metrics_console.detail.load_error")}</AlertTitle>
        </Alert>
      </StatusDialog>
    );
  }
  if (!metric) {
    return (
      <StatusDialog onClose={onClose}>
        <CenteredSpinner className="min-h-40" />
      </StatusDialog>
    );
  }

  return (
    <MetricEditorDialog
      open
      onOpenChange={(open) => (open ? null : onClose())}
      mode="edit"
      initial={graphToDraft(metric)}
      isPending={updateMetric.isPending}
      error={updateMetric.error}
      onSubmit={(draft: MetricDraft) =>
        updateMetric.mutate(draftToGraph(draft), { onSuccess: onClose })
      }
    />
  );
}

/** Minimal modal shell for the edit dialog's loading and error states. */
function StatusDialog({
  onClose,
  children,
}: {
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <Dialog open onOpenChange={(open) => (open ? null : onClose())}>
      <DialogContent className="sm:max-w-md">{children}</DialogContent>
    </Dialog>
  );
}
