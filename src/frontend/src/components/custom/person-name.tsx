import { usePersonName } from "@/queries/person-name";

/** "Owner: name", or nothing when the caller may not learn who that is. */
export function OwnerName({
  personId,
  className,
}: {
  personId: string;
  className?: string;
}) {
  const name = usePersonName(personId).data ?? null;
  if (name === null) return null;

  return <span className={className}>Owner: {name}</span>;
}
