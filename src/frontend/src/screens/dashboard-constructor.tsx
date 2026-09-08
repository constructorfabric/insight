import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";

import {
  queryFieldViolations,
  type QueryDataset,
  type QueryDirection,
  type QueryFilterOp,
  type QueryGrain,
} from "@/api/query-client";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { QueryResults } from "@/components/widgets/query-console/query-results";
import {
  AGGREGATE_FNS,
  FILTER_OPS,
  GRAINS,
  buildQuery,
  emptyAggregate,
  emptyFilter,
  initialDraft,
  operandsOf,
  orderableColumns,
  type AggregateDraft,
  type AggregateFn,
  type FilterDraft,
  type QueryDraft,
} from "@/lib/dashboard-constructor/build-request";
import { apiErrorReason } from "@/lib/query-console/api-error";
import type { ResultRow } from "@/lib/query-console/result-shape";
import { useDatasets, useRunQuery } from "@/queries/query";

export function DashboardConstructorBody() {
  const { t } = useTranslation();
  const datasets = useDatasets();

  if (datasets.isPending) return <CenteredSpinner className="min-h-[60vh]" />;
  if (datasets.isError || datasets.data.length === 0) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label={t("dashboard_constructor.datasets_error")}
          onRetry={() => void datasets.refetch()}
        />
      </div>
    );
  }
  return <Constructor datasets={datasets.data} />;
}

function Constructor({ datasets }: { datasets: QueryDataset[] }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<QueryDraft>(() => initialDraft(datasets[0], new Date()));
  const dataset = useMemo(
    () => datasets.find((d) => d.key === draft.dataset) ?? datasets[0],
    [datasets, draft.dataset],
  );
  const run = useRunQuery();

  const update = (patch: Partial<QueryDraft>) => setDraft((prev) => ({ ...prev, ...patch }));
  const filterFields = [
    ...dataset.dimensions.map((d) => d.field),
    ...dataset.measurables.map((m) => m.field),
  ];

  const switchDataset = (key: string) => {
    const next = datasets.find((d) => d.key === key);
    if (next) setDraft(initialDraft(next, new Date()));
  };

  const submit = () => {
    run.mutate(buildQuery(draft, dataset));
  };

  const violations = queryFieldViolations(run.error);
  const rows: ResultRow[] = useMemo(() => {
    if (!run.data) return [];
    const names = run.data.columns.map((column) => column.name);
    return run.data.rows.map((row) =>
      Object.fromEntries(names.map((name, index) => [name, row[index]])),
    );
  }, [run.data]);

  return (
    <div className="flex flex-col gap-6 p-4 md:p-6">
      <div>
        <h1 className="text-lg font-semibold tracking-tight">{t("dashboard_constructor.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("dashboard_constructor.subtitle")}</p>
      </div>

      <section className="grid gap-4 rounded-lg border p-4 md:grid-cols-2 lg:grid-cols-4">
        <Field label={t("dashboard_constructor.dataset")}>
          <Select value={draft.dataset} onValueChange={(value) => value && switchDataset(value)}>
            <SelectTrigger aria-label={t("dashboard_constructor.dataset")}>
              <SelectValue>{draft.dataset}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {datasets.map((d) => (
                <SelectItem key={d.key} value={d.key}>
                  {d.key}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <Field label={t("dashboard_constructor.from")}>
          <Input type="date" value={draft.from} onChange={(e) => update({ from: e.target.value })} />
        </Field>
        <Field label={t("dashboard_constructor.to")}>
          <Input type="date" value={draft.to} onChange={(e) => update({ to: e.target.value })} />
        </Field>
        <Field label={t("dashboard_constructor.limit")}>
          <Input
            type="number"
            min={1}
            max={dataset.limits.max_limit}
            value={draft.limit}
            onChange={(e) => update({ limit: e.target.value })}
          />
        </Field>
      </section>

      <section className="grid gap-4 rounded-lg border p-4 lg:grid-cols-2">
        <div className="flex flex-col gap-2">
          <Label>{t("dashboard_constructor.group_by")}</Label>
          <div className="flex flex-wrap gap-3">
            {dataset.dimensions.map((dimension) => {
              const checked = draft.dimensions.includes(dimension.field);
              return (
                <label key={dimension.field} className="flex items-center gap-2 text-sm">
                  <Checkbox
                    checked={checked}
                    onCheckedChange={(next) =>
                      update({
                        dimensions: next
                          ? [...draft.dimensions, dimension.field]
                          : draft.dimensions.filter((f) => f !== dimension.field),
                      })
                    }
                  />
                  <span className="font-mono text-xs">{dimension.field}</span>
                </label>
              );
            })}
          </div>
        </div>
        <div className="flex flex-col gap-2">
          <Label>{t("dashboard_constructor.time_axis")}</Label>
          <div className="flex items-center gap-3">
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={draft.timeAxis}
                onCheckedChange={(next) => update({ timeAxis: Boolean(next) })}
              />
              <span>{t("dashboard_constructor.bucket_by_time")}</span>
            </label>
            <Select
              value={draft.grain}
              onValueChange={(value) => value && update({ grain: value as QueryGrain })}
              disabled={!draft.timeAxis}
            >
              <SelectTrigger size="sm" aria-label={t("dashboard_constructor.grain")}>
                <SelectValue>{draft.grain}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                {GRAINS.map((grain) => (
                  <SelectItem key={grain} value={grain}>
                    {grain}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </section>

      <section className="flex flex-col gap-3 rounded-lg border p-4">
        <div className="flex items-center justify-between">
          <Label>{t("dashboard_constructor.aggregates")}</Label>
          <Button
            variant="outline"
            size="sm"
            disabled={draft.aggregates.length >= dataset.limits.max_aggregates}
            onClick={() => update({ aggregates: [...draft.aggregates, emptyAggregate()] })}
          >
            <Plus className="size-4" /> {t("dashboard_constructor.add")}
          </Button>
        </div>
        {draft.aggregates.map((aggregate, index) => (
          <AggregateRow
            key={aggregate.id}
            aggregate={aggregate}
            dataset={dataset}
            filterFields={filterFields}
            violation={violationFor(violations, `aggregates[${index}]`)}
            onChange={(next) =>
              update({
                aggregates: draft.aggregates.map((a) => (a.id === next.id ? next : a)),
              })
            }
            onRemove={() =>
              update({ aggregates: draft.aggregates.filter((a) => a.id !== aggregate.id) })
            }
          />
        ))}
      </section>

      <section className="flex flex-col gap-3 rounded-lg border p-4">
        <div className="flex items-center justify-between">
          <Label>{t("dashboard_constructor.filters")}</Label>
          <Button
            variant="outline"
            size="sm"
            disabled={
              filterFields.length === 0 || draft.filters.length >= dataset.limits.max_filters
            }
            onClick={() => update({ filters: [...draft.filters, emptyFilter(filterFields[0])] })}
          >
            <Plus className="size-4" /> {t("dashboard_constructor.add")}
          </Button>
        </div>
        {draft.filters.map((filter, index) => (
          <FilterRow
            key={filter.id}
            filter={filter}
            fields={filterFields}
            violation={violationFor(violations, `filters[${index}]`)}
            onChange={(next) =>
              update({ filters: draft.filters.map((f) => (f.id === next.id ? next : f)) })
            }
            onRemove={() => update({ filters: draft.filters.filter((f) => f.id !== filter.id) })}
          />
        ))}
      </section>

      <section className="flex flex-wrap items-end gap-4 rounded-lg border p-4">
        <Field label={t("dashboard_constructor.order_by")}>
          <Select
            value={draft.orderBy || "__none__"}
            onValueChange={(value) => update({ orderBy: value === "__none__" || !value ? "" : value })}
          >
            <SelectTrigger size="sm" aria-label={t("dashboard_constructor.order_by")}>
              <SelectValue>{draft.orderBy || t("dashboard_constructor.order_default")}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">{t("dashboard_constructor.order_default")}</SelectItem>
              {orderableColumns(draft).map((column) => (
                <SelectItem key={column} value={column}>
                  {column}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <Field label={t("dashboard_constructor.direction")}>
          <Select
            value={draft.orderDir}
            onValueChange={(value) => value && update({ orderDir: value as QueryDirection })}
          >
            <SelectTrigger size="sm" aria-label={t("dashboard_constructor.direction")}>
              <SelectValue>{draft.orderDir}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="asc">asc</SelectItem>
              <SelectItem value="desc">desc</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Button onClick={submit} disabled={run.isPending || draft.aggregates.length === 0}>
          {run.isPending ? t("dashboard_constructor.running") : t("dashboard_constructor.run")}
        </Button>
      </section>

      {run.isError ? (
        <div role="alert" className="rounded-lg border border-destructive/40 bg-destructive/5 p-4 text-sm">
          <p className="font-medium">{apiErrorReason(run.error, t("dashboard_constructor.run_failed"))}</p>
          {violations.length > 1 ? (
            <ul className="mt-2 list-disc ps-5 text-muted-foreground">
              {violations.slice(1).map((violation) => (
                <li key={violation.field}>
                  <code className="text-xs">{violation.field}</code> — {violation.description}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}

      {run.data ? (
        <section className="flex flex-col gap-3 rounded-lg border p-4">
          <div className="flex items-center gap-2">
            <Label>{t("dashboard_constructor.results")}</Label>
            <span className="text-xs text-muted-foreground">
              {t("dashboard_constructor.row_count", { count: run.data.rows.length })}
            </span>
            {run.data.flags.truncated ? (
              <Badge variant="secondary">{t("dashboard_constructor.truncated")}</Badge>
            ) : null}
          </div>
          <QueryResults rows={rows} />
        </section>
      ) : null}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label>{label}</Label>
      {children}
    </div>
  );
}

function violationFor(
  violations: ReturnType<typeof queryFieldViolations>,
  prefix: string,
): string | null {
  const hit = violations.find((v) => v.field === prefix || v.field.startsWith(`${prefix}.`));
  return hit ? hit.description : null;
}

function AggregateRow({
  aggregate,
  dataset,
  filterFields,
  violation,
  onChange,
  onRemove,
}: {
  aggregate: AggregateDraft;
  dataset: QueryDataset;
  filterFields: string[];
  violation: string | null;
  onChange: (next: AggregateDraft) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const measurables = dataset.measurables.map((m) => m.field);
  const needsField = aggregate.fn !== "count";
  return (
    <div className="flex flex-col gap-2 rounded-md border p-3">
      <div className="flex flex-wrap items-end gap-3">
        <Field label={t("dashboard_constructor.name")}>
          <Input
            value={aggregate.name}
            onChange={(e) => onChange({ ...aggregate, name: e.target.value })}
            className="w-40 font-mono text-xs"
          />
        </Field>
        <Field label={t("dashboard_constructor.function")}>
          <Select
            value={aggregate.fn}
            onValueChange={(value) =>
              value &&
              onChange({
                ...aggregate,
                fn: value as AggregateFn,
                field: value === "count" ? "" : aggregate.field || measurables[0] || "",
              })
            }
          >
            <SelectTrigger size="sm" aria-label={t("dashboard_constructor.function")}>
              <SelectValue>{aggregate.fn}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {AGGREGATE_FNS.map((fn) => (
                <SelectItem key={fn} value={fn}>
                  {fn}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        {needsField ? (
          <Field label={t("dashboard_constructor.measurable")}>
            <Select
              value={aggregate.field}
              onValueChange={(value) => value && onChange({ ...aggregate, field: value })}
            >
              <SelectTrigger size="sm" aria-label={t("dashboard_constructor.measurable")}>
                <SelectValue>{aggregate.field}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                {measurables.map((field) => (
                  <SelectItem key={field} value={field}>
                    {field}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        ) : null}
        <Button
          variant="ghost"
          size="sm"
          onClick={() =>
            onChange({
              ...aggregate,
              filter: aggregate.filter ? null : emptyFilter(filterFields[0] ?? ""),
            })
          }
        >
          {aggregate.filter
            ? t("dashboard_constructor.drop_condition")
            : t("dashboard_constructor.add_condition")}
        </Button>
        <Button variant="ghost" size="icon" aria-label={t("dashboard_constructor.remove")} onClick={onRemove}>
          <Trash2 className="size-4" />
        </Button>
      </div>
      {aggregate.filter ? (
        <FilterRow
          filter={aggregate.filter}
          fields={filterFields}
          violation={null}
          onChange={(next) => onChange({ ...aggregate, filter: next })}
          onRemove={() => onChange({ ...aggregate, filter: null })}
        />
      ) : null}
      {violation ? <p className="text-xs text-destructive">{violation}</p> : null}
    </div>
  );
}

function FilterRow({
  filter,
  fields,
  violation,
  onChange,
  onRemove,
}: {
  filter: FilterDraft;
  fields: string[];
  violation: string | null;
  onChange: (next: FilterDraft) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const operands = operandsOf(filter.op);
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-end gap-3">
        <Field label={t("dashboard_constructor.field")}>
          <Select value={filter.field} onValueChange={(value) => value && onChange({ ...filter, field: value })}>
            <SelectTrigger size="sm" aria-label={t("dashboard_constructor.field")}>
              <SelectValue>{filter.field}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {fields.map((field) => (
                <SelectItem key={field} value={field}>
                  {field}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <Field label={t("dashboard_constructor.operator")}>
          <Select
            value={filter.op}
            onValueChange={(value) => value && onChange({ ...filter, op: value as QueryFilterOp })}
          >
            <SelectTrigger size="sm" aria-label={t("dashboard_constructor.operator")}>
              <SelectValue>{filter.op}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {FILTER_OPS.map((op) => (
                <SelectItem key={op} value={op}>
                  {op}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        {operands.value ? (
          <Field
            label={
              filter.op === "in"
                ? t("dashboard_constructor.values_csv")
                : filter.op === "like" || filter.op === "match"
                  ? t("dashboard_constructor.pattern")
                  : t("dashboard_constructor.value")
            }
          >
            <Input
              value={filter.value}
              onChange={(e) => onChange({ ...filter, value: e.target.value })}
              className="w-56 font-mono text-xs"
            />
          </Field>
        ) : null}
        {operands.range ? (
          <>
            <Field label={t("dashboard_constructor.low")}>
              <Input
                value={filter.low}
                onChange={(e) => onChange({ ...filter, low: e.target.value })}
                className="w-28 font-mono text-xs"
              />
            </Field>
            <Field label={t("dashboard_constructor.high")}>
              <Input
                value={filter.high}
                onChange={(e) => onChange({ ...filter, high: e.target.value })}
                className="w-28 font-mono text-xs"
              />
            </Field>
          </>
        ) : null}
        <Button variant="ghost" size="icon" aria-label={t("dashboard_constructor.remove")} onClick={onRemove}>
          <Trash2 className="size-4" />
        </Button>
      </div>
      {violation ? <p className="text-xs text-destructive">{violation}</p> : null}
    </div>
  );
}
