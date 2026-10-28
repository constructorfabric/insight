import type { PersonSummary } from "@/api/identity-client";
import { PersonPicker } from "@/components/portal/person-picker";
import { Spinner } from "@/components/ui/spinner";
import { normalizePersonId } from "@/lib/metrics/entity";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { useAdmins } from "@/queries/person-roles";

/**
 * Finds a person who holds the administrator role now. The service checks the
 * role again on the change this feeds, so this only narrows what is offered.
 */
export function AdminPicker({
  exclude = [],
  onPick,
}: {
  /** Persons not to offer, such as the owner or those already chosen. */
  exclude?: readonly string[];
  onPick: (person: PersonSummary) => void;
}) {
  const admins = useAdmins();

  if (admins.isPending) return <Spinner className="mx-auto size-4" />;

  // INVARIANT: a full page may leave administrators unread, and offering part
  // of them would answer "no match" for someone who is one.
  if (admins.isError || !admins.data.complete) {
    return (
      <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
        Couldn't read the administrators.
      </p>
    );
  }

  const excluded = new Set(exclude.map(normalizePersonId));
  const offered = new Set(
    [...admins.data.personIds].filter((id) => !excluded.has(id))
  );

  return <PersonPicker onlyIds={offered} onPick={onPick} />;
}
