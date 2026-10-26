import type { ReactNode } from "react";

import { usePersonName } from "@/queries/person-name";

/**
 * A person by the name they go by. Read through the tenant search it falls
 * back to the id; read through the roster, a person the caller may not see
 * renders as nothing.
 */
export function PersonName({ personId }: { personId: string }) {
  const name = useShownName(personId);

  return name;
}

/** "Owner: name", or nothing when the caller may not learn who that is. */
export function OwnerName({
  personId,
  className,
}: {
  personId: string;
  className?: string;
}) {
  const name = useShownName(personId);
  if (name === null) return null;

  return <span className={className}>Owner: {name}</span>;
}

function useShownName(personId: string): ReactNode {
  const name = usePersonName(personId);

  if (name.isPending) return null;
  if (name.data) return name.data;

  return name.isError ? null : personId;
}
