import type { ReactNode } from "react";

import type { DefinitionKind } from "@/api/custom-client";
import {
  DefinitionCount,
  MoreDefinitions,
  type Paging,
} from "@/components/custom/definition-paging";
import { DefinitionSearch } from "@/components/custom/definition-search";
import { RemoveDefinition } from "@/components/custom/remove-definition";
import { RenameDefinition } from "@/components/custom/rename-definition";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { TEXT_BODY, TEXT_HEADING, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * A catalogue page: every definition of one kind, each in a card that renders
 * its own body. The list endpoint returns names only, so each row fetches its
 * own definition — which is also what keeps a broken one from blanking the
 * page.
 */
export function DefinitionList({
  title,
  blurb,
  names,
  isLoading,
  isError,
  onRetry,
  emptyLabel,
  renderRow,
  search,
  paging,
}: {
  title: string;
  blurb: string;
  names: string[] | undefined;
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
  emptyLabel: string;
  renderRow: (name: string) => ReactNode;
  search?: { label: string; value: string; onChange: (needle: string) => void };
  paging?: Paging;
}) {
  return (
    <>
      <header className="mb-3">
        <h1 className={TEXT_TITLE}>{title}</h1>
        <p className={cn(TEXT_BODY, "text-muted-foreground")}>{blurb}</p>
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
      ) : !names ? null : names.length === 0 ? (
        <ComingSoon variant="card" state="empty" label={emptyLabel} />
      ) : (
        <>
          <ul className="grid gap-4 @3xl:grid-cols-2">
            {names.map((name) => (
              <li key={name}>{renderRow(name)}</li>
            ))}
          </ul>
          {paging ? <MoreDefinitions {...paging} /> : null}
        </>
      )}
    </>
  );
}

/** One definition's card, with its name as the identifier it is. */
export function DefinitionCard({
  name,
  kind,
  children,
}: {
  name: string;
  kind: DefinitionKind;
  children: ReactNode;
}) {
  return (
    <Card className="h-full">
      <CardHeader className="flex flex-row items-center gap-2">
        <CardTitle className={cn(TEXT_HEADING, "min-w-0 truncate font-mono")}>
          {name}
        </CardTitle>
        <span className="ms-auto flex shrink-0 items-center gap-1">
          <RenameDefinition kind={kind} name={name} />
          <RemoveDefinition kind={kind} name={name} />
        </span>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}
