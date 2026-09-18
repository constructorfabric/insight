import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import type { DefinitionKind, Holder } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { dependentsQuery } from "@/queries/custom";
import { TEXT_BODY, TEXT_HEADING } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** Where a holder of each kind is looked at. */
const PAGE: Record<Holder["kind"], string> = {
  metrics: "/portal/custom/metrics/$name",
  widgets: "/portal/custom/widgets/$name",
  dashboards: "/portal/custom/$name",
};

/**
 * What names this definition, so a reader sees what a removal would break
 * before they ask for one.
 */
export function HeldBy({
  kind,
  name,
  title = "Used by",
  empty = "Nothing draws it yet.",
}: {
  kind: DefinitionKind;
  name: string;
  title?: string;
  empty?: string;
}) {
  const held = useQuery(dependentsQuery(kind, name));

  return (
    <Held
      title={title}
      empty={empty}
      pending={held.isPending}
      error={held.error}
      holders={held.data}
    />
  );
}

/** The same card over holders read some other way, as a dataset's are. */
export function Held({
  title,
  empty,
  pending,
  error,
  holders,
}: {
  title: string;
  empty: string;
  pending: boolean;
  error: unknown;
  holders: Holder[] | undefined;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>{title}</CardTitle>
      </CardHeader>
      <CardContent>
        {pending ? (
          <CenteredSpinner className="min-h-16" />
        ) : error ? (
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            {refusal(error, "Couldn't read what uses it.")}
          </p>
        ) : !holders || holders.length === 0 ? (
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>{empty}</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {holders.map((holder) => (
              <li key={`${holder.kind}/${holder.name}`} className={TEXT_BODY}>
                <Link
                  to={PAGE[holder.kind]}
                  params={{ name: holder.name }}
                  className="font-mono underline decoration-dotted underline-offset-4"
                >
                  {holder.name}
                </Link>{" "}
                <span className="text-muted-foreground">{holder.kind}</span>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}
