import { useQuery } from "@tanstack/react-query";

import { metricQuery } from "@/queries/custom";

/** A metric by its name, or by its id until the name is read or when it cannot be. */
export function MetricName({ id }: { id: string }) {
  const metric = useQuery({ ...metricQuery(id), enabled: id !== "" });

  return <>{metric.data?.name ?? id}</>;
}
