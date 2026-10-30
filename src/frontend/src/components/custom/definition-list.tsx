import type { ReactNode } from "react";

import type { DefinitionKind, Listed } from "@/api/custom-client";
import {
  DefinitionCount,
  MoreDefinitions,
  type Paging,
} from "@/components/custom/definition-paging";
import { DefinitionSearch } from "@/components/custom/definition-search";
import { EditLink, PreviewLink } from "@/components/custom/editor/edit-link";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { TEXT_BODY, TEXT_HEADING, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * A catalogue page: every definition of one kind, each in a card that renders
 * its own body. The list endpoint returns ids and names only, so each row
 * fetches its own definition — which is also what keeps a broken one from
 * blanking the page.
 */
export function DefinitionList({
  "data-testid": testId,
  title,
  blurb,
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
  "data-testid"?: string;
  title: string;
  blurb: string;
  items: Listed[] | undefined;
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
  emptyLabel: string;
  renderRow: (item: Listed) => ReactNode;
  search?: { label: string; value: string; onChange: (needle: string) => void };
  paging?: Paging;
  /** What starts a new one of this kind, where one may be written by hand. */
  create?: ReactNode;
}) {
  return (
    <>
      <header
        data-testid={testId}
        className="mb-3 flex flex-wrap items-start gap-3"
      >
        <div className="min-w-0 grow">
          <h1 className={TEXT_TITLE}>{title}</h1>
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>{blurb}</p>
        </div>
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
          <ul className="grid gap-4 @3xl:grid-cols-2">
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

export function DefinitionCard({
  name,
  actions,
  children,
}: {
  name: string;
  actions: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card className="h-full">
      <CardHeader className="flex flex-row items-center gap-2">
        <CardTitle className={cn(TEXT_HEADING, "min-w-0 truncate")}>
          {name}
        </CardTitle>
        <span className="ms-auto flex shrink-0 items-center gap-1">
          {actions}
        </span>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

/** What a card offers: opening the editor, where renaming and removal live too. */
export function DefinitionActions({
  kind,
  id,
  name,
}: {
  kind: DefinitionKind;
  id: string;
  name: string;
}) {
  return (
    <>
      <EditLink kind={kind} id={id} name={name} />
      {kind === "dashboards" ? null : (
        <PreviewLink kind={kind} id={id} name={name} />
      )}
    </>
  );
}
