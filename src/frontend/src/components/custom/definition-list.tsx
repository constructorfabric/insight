import type { ReactNode } from "react";

import {
  DefinitionCount,
  MoreDefinitions,
  type Paging,
} from "@/components/custom/definition-paging";
import { DefinitionSearch } from "@/components/custom/definition-search";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { TEXT_TITLE } from "@/lib/type-scale";

/** A catalogue page: every definition of one kind, a card each. */
export function DefinitionList<T extends { id: string }>({
  title,
  items,
  isLoading,
  isError,
  onRetry,
  emptyLabel,
  renderRow,
  search,
  paging,
  create,
}: {
  title: string;
  items: T[] | undefined;
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
  emptyLabel: string;
  renderRow: (item: T) => ReactNode;
  search?: { label: string; value: string; onChange: (needle: string) => void };
  paging?: Paging;
  /** What starts a new one of this kind, where one may be written by hand. */
  create?: ReactNode;
}) {
  return (
    <>
      <header className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <h1 className={TEXT_TITLE}>{title}</h1>
        {create}
      </header>
      {search || paging ? (
        <div className="mb-4 flex flex-wrap items-center gap-3">
          {search ? (
            <DefinitionSearch
              label={search.label}
              value={search.value}
              onChange={search.onChange}
            />
          ) : null}
          {paging ? (
            <DefinitionCount
              total={paging.total}
              noun={title.toLowerCase()}
              searching={paging.searching}
            />
          ) : null}
        </div>
      ) : null}
      {isLoading ? (
        <CenteredSpinner className="min-h-40" />
      ) : isError ? (
        <ComingSoon
          variant="card"
          state="error"
          label={`Couldn't load the ${title.toLowerCase()}.`}
          onRetry={onRetry}
        />
      ) : !items ? null : items.length === 0 ? (
        <ComingSoon variant="card" state="empty" label={emptyLabel} />
      ) : (
        <>
          <ul className="grid gap-3 @xl:grid-cols-2 @5xl:grid-cols-3">
            {items.map((item) => (
              <li key={item.id}>{renderRow(item)}</li>
            ))}
          </ul>
          {paging ? <MoreDefinitions {...paging} /> : null}
        </>
      )}
    </>
  );
}
