/**
 * Reporting the server's answer to an identity correction, the same way from
 * every surface that fires one.
 *
 * A verb answers with three counters plus a per-account outcome, and that
 * vocabulary is open by contract — so success is only ever "every account the
 * call named ended up decided", never "no refusals". A refusal means the
 * account kept its binding, which is a decision the operator still has to
 * take, so the caller is told to keep its verbs on screen rather than being
 * handed a generic failure.
 */
import { useTranslation } from "react-i18next";

import type { CorrectionResponse } from "@/api/identity-client";
import { toast } from "@/components/ui/sonner";
import { fullyDecided, refusedCount } from "@/lib/identities/outcomes";

/** Long enough to read a UUID off and paste it somewhere. */
const MINTED_ID_TOAST_MS = 15_000;

/**
 * Report one answer and say whether it settled everything it named.
 *
 * `false` is the caller's cue to keep its window and its verbs on screen, and
 * to show the counters verbatim — see `OutcomeAlert`.
 */
export function useCorrectionReport(): (result: CorrectionResponse) => boolean {
  const { t } = useTranslation();
  return (result: CorrectionResponse) => {
    if (!fullyDecided(result)) {
      const refused = refusedCount(result);
      toast.error(
        refused > 0
          ? t("identities.outcomes.toast_refused", { count: refused })
          : t("identities.dialogs.failed"),
      );
      return false;
    }

    const message =
      result.applied > 0
        ? t("identities.outcomes.toast_applied", { count: result.applied })
        : t("identities.outcomes.toast_already");
    // The minted person's id is the one thing a detach reports that nothing
    // else on the page can name yet, and the window that held it is closing —
    // so the toast carrying it stays up long enough to be copied out.
    toast.success(message, {
      description: result.new_person_id
        ? `${t("identities.outcomes.new_person")} ${result.new_person_id}`
        : undefined,
      duration: result.new_person_id ? MINTED_ID_TOAST_MS : undefined,
    });
    return true;
  };
}
