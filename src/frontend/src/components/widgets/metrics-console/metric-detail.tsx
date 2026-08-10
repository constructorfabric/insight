import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { MetricPreview } from "@/components/widgets/metrics-console/metric-preview";
import { apiErrorReason } from "@/lib/query-console/api-error";
import {
  useCustomMetric,
  useCustomMetricPreview,
} from "@/queries/custom-metrics";
import { Pencil, Play, Trash2, TriangleAlert } from "lucide-react";

export interface MetricDetailProps {
  metricKey: string;
  onEdit: (metricKey: string) => void;
  onDelete: (metricKey: string) => void;
}

function splitIds(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((token) => token.trim())
    .filter((token) => token !== "");
}

export function MetricDetail({ metricKey, onEdit, onDelete }: MetricDetailProps) {
  const { t } = useTranslation();
  const { data: metric, isPending, isError } = useCustomMetric(metricKey);
  const preview = useCustomMetricPreview(metricKey);
  const [entityIds, setEntityIds] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");

  if (isPending) return <CenteredSpinner className="min-h-60" />;

  if (isError || !metric) {
    return (
      <Alert variant="destructive">
        <TriangleAlert />
        <AlertTitle>{t("metrics_console.detail.load_error")}</AlertTitle>
      </Alert>
    );
  }

  const ids = splitIds(entityIds);
  const canPreview =
    ids.length > 0 && from.trim() !== "" && to.trim() !== "" && !preview.isPending;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-semibold tracking-tight">
            {metric.label}
          </h2>
          <div className="mt-1 flex flex-wrap gap-1.5">
            <Badge variant="secondary" className="font-mono">
              {metric.metric_key}
            </Badge>
            <Badge variant="outline">{metric.computation}</Badge>
            <Badge variant="outline">{metric.entity_type}</Badge>
            <Badge variant="outline">{metric.format}</Badge>
          </div>
          {metric.description ? (
            <p className="mt-2 text-sm text-muted-foreground">
              {metric.description}
            </p>
          ) : null}
        </div>
        <div className="flex gap-1.5">
          <Button
            variant="outline"
            size="sm"
            onClick={() => onEdit(metricKey)}
          >
            <Pencil />
            {t("metrics_console.detail.edit")}
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={() => onDelete(metricKey)}
          >
            <Trash2 />
            {t("metrics_console.detail.delete")}
          </Button>
        </div>
      </div>

      <pre className="overflow-x-auto rounded-md border bg-muted/40 p-3 font-mono text-xs">
        {metric.observation_sql}
      </pre>

      <div className="flex flex-wrap items-end gap-2">
        <div className="flex flex-col gap-1.5">
          <Label
            htmlFor="metric-preview-ids"
            className="text-xs font-medium text-muted-foreground"
          >
            {t("metrics_console.detail.entity_ids_label")}
          </Label>
          <Input
            id="metric-preview-ids"
            value={entityIds}
            onChange={(event) => setEntityIds(event.target.value)}
            placeholder={t("metrics_console.detail.entity_ids_placeholder")}
            className="w-72"
            autoComplete="off"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label
            htmlFor="metric-preview-from"
            className="text-xs font-medium text-muted-foreground"
          >
            {t("metrics_console.detail.from_label")}
          </Label>
          <Input
            id="metric-preview-from"
            value={from}
            onChange={(event) => setFrom(event.target.value)}
            placeholder="2026-01-01"
            className="w-36"
            autoComplete="off"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label
            htmlFor="metric-preview-to"
            className="text-xs font-medium text-muted-foreground"
          >
            {t("metrics_console.detail.to_label")}
          </Label>
          <Input
            id="metric-preview-to"
            value={to}
            onChange={(event) => setTo(event.target.value)}
            placeholder="2026-01-31"
            className="w-36"
            autoComplete="off"
          />
        </div>
        <Button
          onClick={() =>
            preview.mutate({
              entityType: metric.entity_type,
              entityIds: ids,
              from: from.trim(),
              to: to.trim(),
            })
          }
          disabled={!canPreview}
        >
          <Play />
          {t("metrics_console.detail.preview")}
        </Button>
      </div>

      {preview.isPending ? <CenteredSpinner className="min-h-40" /> : null}

      {preview.isError ? (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>{t("metrics_console.detail.preview_error")}</AlertTitle>
          <AlertDescription>
            {apiErrorReason(
              preview.error,
              t("metrics_console.detail.preview_error_generic")
            )}
          </AlertDescription>
        </Alert>
      ) : null}

      {preview.isSuccess ? <MetricPreview result={preview.data} /> : null}
    </div>
  );
}
