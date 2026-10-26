import { useQuery } from "@tanstack/react-query";

import { useIsAdmin } from "@/queries/identity-me";
import { metricQuery } from "@/queries/custom";

export function MetricName({ id }: { id: string }) {
  const { isAdmin, isPending } = useIsAdmin();
  const member = !isPending && !isAdmin;
  const metric = useQuery({ ...metricQuery(id), enabled: id !== "" && !member });

  if (metric.data) return <>{metric.data.name}</>;
  return metric.isError ? <>{id}</> : null;
}
