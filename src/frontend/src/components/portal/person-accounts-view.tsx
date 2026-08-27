/**
 * The console's second mode: the roster, and the window one person is worked
 * in.
 *
 * The review queue only ever shows what automation could not decide, so a
 * settled binding is invisible — and the questions that actually arrive are
 * about settled bindings ("this person's work looks split", "that account is
 * not theirs"). The verbs already accept those accounts; this is the door to
 * them.
 *
 * Entered through a person rather than through a list of every binding: the
 * roster is browsable here, but a decision still starts from whose accounts
 * these are, so it stays attached to the question that prompted it.
 *
 * The roster stays on screen while a person is open, exactly as the account
 * listing does — the person rides in `?person=`, the window over it is
 * {@link PersonDialog}, and closing it IS the way back.
 */
import { useState } from "react";

import type { PersonSummary } from "@/api/identity-client";
import { PersonDialog } from "@/components/portal/person-dialog";
import { PersonPicker } from "@/components/portal/person-picker";
import { usePortalSearch, useSetPortalSearch } from "@/lib/portal/portal-search";

export function PersonAccountsView() {
  const { person, find } = usePortalSearch();
  const setSearch = useSetPortalSearch();
  // The URL owns which person is open; this remembers the card the operator
  // picked, so the window names them. Arriving by link there is no card —
  // search resolves values, not ids — and the id stands alone, honestly.
  const [picked, setPicked] = useState<PersonSummary | null>(null);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-4">
      <PersonPicker
        browseWhenEmpty
        asSurface
        // The roster's terms live in the URL: a reader who comes back to this
        // mode should not have to find the same person twice. `replace`,
        // because typing is not a place to go back to.
        initialQuery={find ?? ""}
        onSettled={(query) =>
          setSearch({ find: query || undefined }, { replace: true })
        }
        onPick={(next: PersonSummary) => {
          setPicked(next);
          setSearch({ person: next.person_id });
        }}
      />
      <PersonDialog
        personId={person}
        card={picked}
        onClose={() => setSearch({ person: undefined })}
      />
    </div>
  );
}
