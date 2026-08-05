/**
 * Shape a saved-query `/run` result (untyped JSON rows, per-query dynamic
 * columns) into what the console renders: a table over inferred columns, and,
 * when the rows are chartable, an auto-chart model.
 *
 * Pure over the row array so the heuristics are unit-testable without a
 * rendered component or a backend.
 */

export type ResultRow = Record<string, unknown>;

/** Beyond this many rows a categorical bar chart is unreadable — table only. */
const MAX_CHART_ROWS = 50;

/** Column names in first-seen order across the union of row keys. */
export function inferColumns(rows: ResultRow[]): string[] {
  const columns: string[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    for (const key of Object.keys(row)) {
      if (!seen.has(key)) {
        seen.add(key);
        columns.push(key);
      }
    }
  }
  return columns;
}

export interface AutoChartModel {
  /** Categorical column used for the x-axis. */
  labelKey: string;
  /** Numeric columns rendered as bar series. */
  valueKeys: string[];
}

/**
 * A result is chartable when it has at least one row (but not so many that a
 * categorical chart turns to noise), *exactly one* categorical column for the
 * x-axis, and at least one numeric column for the bars. More than one
 * categorical column is ambiguous (which is the axis?), so the console renders
 * the table alone.
 */
export function inferChartModel(rows: ResultRow[]): AutoChartModel | null {
  if (rows.length === 0 || rows.length > MAX_CHART_ROWS) return null;

  const columns = inferColumns(rows);
  const categorical = columns.filter((column) => !isNumericColumn(rows, column));
  if (categorical.length !== 1) return null;

  const labelKey = categorical[0];
  const valueKeys = columns.filter(
    (column) => column !== labelKey && isNumericColumn(rows, column)
  );
  if (valueKeys.length === 0) return null;

  return { labelKey, valueKeys };
}

/** Coerce a cell to a finite number, or `null` when it is not numeric. */
export function toNumber(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

/** Render any cell as a table string; objects/arrays are JSON, null is a dash. */
export function formatCell(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

/**
 * A column is numeric when every present value across the rows coerces to a
 * finite number and at least one value is present. `null`/absent cells are
 * ignored (a numeric column with a gap is still numeric); an all-null column is
 * not numeric (nothing to plot). ClickHouse serializes wide integers as JSON
 * strings, so string-encoded numbers count.
 */
function isNumericColumn(rows: ResultRow[], column: string): boolean {
  let hasValue = false;
  for (const row of rows) {
    const cell = row[column];
    if (cell === null || cell === undefined) continue;
    if (toNumber(cell) === null) return false;
    hasValue = true;
  }
  return hasValue;
}
