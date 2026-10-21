import { usePersonName } from "@/queries/person-name";

/** A person by the name they go by, or by their id once no name is found. */
export function PersonName({ personId }: { personId: string }) {
  const name = usePersonName(personId);

  if (name.isPending) return null;

  return <>{name.data ?? personId}</>;
}
