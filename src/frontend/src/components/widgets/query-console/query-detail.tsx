import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { QueryResults } from "@/components/widgets/query-console/query-results";
import { apiErrorReason } from "@/lib/query-console/api-error";
import { useRunSavedQuery, useSavedQuery } from "@/queries/saved-queries";
import { Pencil, Play, Trash2, TriangleAlert } from "lucide-react";

export interface QueryDetailProps {
  id: string;
  onEdit: (id: string) => void;
  onDelete: (id: string) => void;
}

export function QueryDetail({ id, onEdit, onDelete }: QueryDetailProps) {
  const { t } = useTranslation();
  const { data: query, isPending, isError } = useSavedQuery(id);
  const run = useRunSavedQuery(id);
  const [period, setPeriod] = useState("");

  if (isPending) return <CenteredSpinner className="min-h-60" />;

  if (isError || !query) {
    return (
      <Alert variant="destructive">
        <TriangleAlert />
        <AlertTitle>{t("query_console.detail.load_error")}</AlertTitle>
      </Alert>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-semibold tracking-tight">
            {query.name}
          </h2>
          {query.description ? (
            <p className="text-sm text-muted-foreground">{query.description}</p>
          ) : null}
        </div>
        <div className="flex gap-1.5">
          <Button variant="outline" size="sm" onClick={() => onEdit(id)}>
            <Pencil />
            {t("query_console.detail.edit")}
          </Button>
          <Button variant="destructive" size="sm" onClick={() => onDelete(id)}>
            <Trash2 />
            {t("query_console.detail.delete")}
          </Button>
        </div>
      </div>

      <pre className="overflow-x-auto rounded-md border bg-muted/40 p-3 font-mono text-xs">
        {query.sql}
      </pre>

      <div className="flex flex-wrap items-end gap-2">
        <div className="flex flex-col gap-1.5">
          <label
            htmlFor="query-period"
            className="text-xs font-medium text-muted-foreground"
          >
            {t("query_console.detail.period_label")}
          </label>
          <Input
            id="query-period"
            value={period}
            onChange={(event) => setPeriod(event.target.value)}
            placeholder={t("query_console.detail.period_placeholder")}
            className="w-40"
            autoComplete="off"
          />
        </div>
        <Button
          onClick={() =>
            run.mutate(period.trim() ? { period: period.trim() } : {})
          }
          disabled={run.isPending}
        >
          <Play />
          {t("query_console.detail.run")}
        </Button>
      </div>

      {run.isPending ? <CenteredSpinner className="min-h-40" /> : null}

      {run.isError ? (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>{t("query_console.detail.run_error")}</AlertTitle>
          <AlertDescription>
            {apiErrorReason(
              run.error,
              t("query_console.detail.run_error_generic")
            )}
          </AlertDescription>
        </Alert>
      ) : null}

      {run.isSuccess ? <QueryResults rows={run.data.rows} /> : null}
    </div>
  );
}
