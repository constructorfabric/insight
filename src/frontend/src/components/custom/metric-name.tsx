import { useQuery } from "@tanstack/react-query";

import { metricQuery } from "@/queries/custom";

export function MetricName({ id }: { id: string }) {
  const metric = useQuery({ ...metricQuery(id), enabled: id !== "" });

  if (metric.data) return <>{metric.data.name}</>;
  return metric.isError ? <>{id}</> : null;
}
