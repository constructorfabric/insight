import { usePersonName } from "@/queries/person-name";

/**
 * A person by the name they go by. Read through the tenant search it falls
 * back to the id; read through the roster, a person the caller may not see or
 * who has no name renders as nothing.
 */
export function PersonName({ personId }: { personId: string }) {
  return useShownName(personId);
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

function useShownName(personId: string): string | null {
  const name = usePersonName(personId);

  return name.data ?? null;
}
