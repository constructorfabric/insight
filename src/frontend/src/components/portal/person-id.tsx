/**
 * A person id, always shown and always copyable.
 *
 * A trail of decisions names the same handful of people over and over, and the
 * id is what tells two of them apart when the names do not — it is also what an
 * operator pastes into a search, a query or a ticket.
 */
import { useTranslation } from "react-i18next";

import { CopyValueButton } from "@/components/copy-value-button";

export function PersonId({ id }: { id: string }) {
  const { t } = useTranslation();
  return (
    <span className="inline-flex items-center gap-1">
      <span className="font-mono text-xs select-text">{id}</span>
      <CopyValueButton
        value={id}
        title={t("identities.person.copy_id")}
        copyLabel={t("common.copy")}
        copiedLabel={t("common.copied")}
        errorMessage={t("common.copy_failed")}
      />
    </span>
  );
}
