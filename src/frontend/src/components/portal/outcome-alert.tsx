/**
 * The server's answer to a correction, verbatim — three counters, never a bare
 * "done".
 *
 * Shown where a verb did NOT settle everything it named: those accounts kept
 * their bindings, so the operator still has a decision to take and needs the
 * counters to see it. The toast beside it comes from `useCorrectionReport`.
 */
import { useTranslation } from "react-i18next";

import type { CorrectionResponse } from "@/api/identity-client";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { refusedCount } from "@/lib/identities/outcomes";

export function OutcomeAlert({ outcome }: { outcome: CorrectionResponse }) {
  const { t } = useTranslation();
  const refused = refusedCount(outcome);
  return (
    <Alert variant={refused > 0 ? "destructive" : "default"} role="status">
      <AlertTitle className="flex flex-wrap items-center gap-1.5">
        <Badge variant="secondary">
          {t("identities.outcomes.applied", { count: outcome.applied })}
        </Badge>
        {outcome.already_decided > 0 ? (
          <Badge variant="outline">
            {t("identities.outcomes.already_decided", {
              count: outcome.already_decided,
            })}
          </Badge>
        ) : null}
        {refused > 0 ? (
          <Badge variant="secondary" className="bg-destructive/15 text-destructive">
            {t("identities.outcomes.refused", { count: refused })}
          </Badge>
        ) : null}
      </AlertTitle>
      {outcome.new_person_id ? (
        <AlertDescription className="font-mono text-xs">
          {t("identities.outcomes.new_person")} {outcome.new_person_id}
        </AlertDescription>
      ) : null}
    </Alert>
  );
}
