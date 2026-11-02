import { Search, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import {
  DefinitionCount,
  MoreDefinitions,
  type Paging,
} from "@/components/custom/definition-paging";
import { DefinitionSearch } from "@/components/custom/definition-search";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyActions,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { TEXT_TITLE } from "@/lib/type-scale";

/** A catalogue page: every definition of one kind, a card each. */
export function DefinitionList<T extends { id: string }>({
  title,
  items,
  isLoading,
  isError,
  errorLabel,
  onRetry,
  empty,
  renderRow,
  search,
  paging,
  create,
}: {
  title: string;
  items: T[] | undefined;
  isLoading: boolean;
  isError: boolean;
  /** What the service said when it refused, where it said something. */
  errorLabel?: string;
  onRetry: () => void;
  /** What an empty catalogue shows before anything is searched for. */
  empty: { icon: LucideIcon; title: string; description?: string };
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
          label={errorLabel ?? `Couldn't load the ${title.toLowerCase()}.`}
          onRetry={onRetry}
        />
      ) : !items ? null : items.length === 0 ? (
        (paging?.searching ?? Boolean(search?.value.trim())) ? (
          <NothingFound
            icon={Search}
            title={`No ${title.toLowerCase()} match your search`}
            action={
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => search?.onChange("")}
              >
                Clear search
              </Button>
            }
          />
        ) : (
          <NothingFound {...empty} action={create} />
        )
      ) : (
        <>
          <ul className="grid grid-cols-1 gap-3 @xl:grid-cols-2 @5xl:grid-cols-3">
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

function NothingFound({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon: LucideIcon;
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Icon aria-hidden />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        {description ? (
          <EmptyDescription>{description}</EmptyDescription>
        ) : null}
      </EmptyHeader>
      {action ? <EmptyActions>{action}</EmptyActions> : null}
    </Empty>
  );
}
