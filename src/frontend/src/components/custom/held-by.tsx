import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import type { DefinitionKind } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { dependentsQuery } from "@/queries/custom";
import { TEXT_BODY, TEXT_HEADING } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** Where a holder of each kind is looked at. */
const PAGE = {
  metrics: "/portal/custom/metrics/$name",
  widgets: "/portal/custom/widgets/$name",
  dashboards: "/portal/custom/$name",
} as const;

/**
 * What names this definition, so a reader sees what a removal would break
 * before they ask for one.
 */
export function HeldBy({ kind, name }: { kind: DefinitionKind; name: string }) {
  const held = useQuery(dependentsQuery(kind, name));

  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>Used by</CardTitle>
      </CardHeader>
      <CardContent>
        {held.isPending ? (
          <CenteredSpinner className="min-h-16" />
        ) : held.isError ? (
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            {refusal(held.error, "Couldn't read what uses it.")}
          </p>
        ) : held.data.length === 0 ? (
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            Nothing draws it yet.
          </p>
        ) : (
          <ul className="flex flex-col gap-1">
            {held.data.map((holder) => (
              <li key={`${holder.kind}/${holder.name}`} className={TEXT_BODY}>
                <Link
                  to={PAGE[holder.kind as keyof typeof PAGE]}
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
