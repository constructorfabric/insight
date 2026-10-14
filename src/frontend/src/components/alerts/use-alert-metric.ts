import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import type { ColumnKind } from "@/lib/alerts/condition";
import { columnKinds, metricShape } from "@/lib/alerts/shape";
import { datasetQuery, metricQuery } from "@/queries/custom";

const NO_KINDS: ReadonlyMap<string, ColumnKind> = new Map();

export function useAlertMetric(metric: string) {
  const read = useQuery({ ...metricQuery(metric), enabled: metric !== "" });
  const definition = read.data?.definition;
  const dataset = definition?.dataset ?? "";
  const declared = useQuery({
    ...datasetQuery(dataset),
    enabled: dataset !== "",
  });
  // INVARIANT: over a dataset only the declaration types a plain column, so
  // until it is read, or when it cannot be, no column has a kind.
  const declaration = declared.data?.declaration;
  const typed =
    definition !== undefined && (dataset === "" || declaration !== undefined);
  const kinds = useMemo(
    () =>
      typed && definition
        ? new Map(columnKinds(definition, declaration))
        : NO_KINDS,
    [typed, definition, declaration]
  );

  const settled =
    read.isError ||
    (definition !== undefined &&
      (dataset === "" || declared.isSuccess || declared.isError));

  return {
    read,
    stored: read.data,
    definition,
    kinds,
    typed,
    /** Read as far as it will be: the parts that hang on it can be drawn. */
    settled,
    manyRows: definition ? metricShape(definition).kind !== "one" : false,
  };
}

export type AlertMetric = ReturnType<typeof useAlertMetric>;
