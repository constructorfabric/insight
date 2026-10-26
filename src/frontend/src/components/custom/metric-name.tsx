import { useQuery } from "@tanstack/react-query";

import { useIsAdmin } from "@/queries/identity-me";
import { metricQuery } from "@/queries/custom";

/** A metric by its name, or by its id until the name is read or when it cannot be. */
export function MetricName({ id }: { id: string }) {
  const { isAdmin, isPending } = useIsAdmin();
  const member = !isPending && !isAdmin;
  const metric = useQuery({ ...metricQuery(id), enabled: id !== "" && !member });

  return <>{metric.data?.name ?? id}</>;
}
