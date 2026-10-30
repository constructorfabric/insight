import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRight, Ellipsis, Eye, Pencil, Trash2 } from "lucide-react";
import { useState, type ComponentType, type ReactNode } from "react";

import { ConfirmDialog } from "@/components/confirm-dialog";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { PREVIEWED, type PreviewedKind } from "@/lib/custom/catalogue-paths";
import { changedAgo } from "@/lib/custom/changed-ago";
import { TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { useRemoveDataset, useRemoveDefinition } from "@/queries/custom";

const NOUN: Record<PreviewedKind, string> = {
  metrics: "metric",
  widgets: "widget",
  datasets: "dataset",
};

const REMOVAL_NOTE: Record<PreviewedKind, string> = {
  metrics:
    "Widgets still drawing it keep it from being deleted. This can't be undone.",
  widgets:
    "Dashboards still holding it keep it from being deleted. This can't be undone.",
  datasets: "The records it holds are deleted with it. This can't be undone.",
};

/**
 * One metric, widget or dataset in its catalogue, laid out as a dashboard card
 * is: what it is under its title, when it last changed, and a menu.
 */
export function CatalogueCard({
  kind,
  id,
  name,
  title,
  icon: Icon,
  facts,
  squeezed = 0,
  updatedAt,
}: {
  kind: PreviewedKind;
  id: string;
  name: string;
  title: string;
  icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  /** What it is, each part a short phrase. */
  facts: ReactNode[];
  /** The one fact cut short when the line runs out of room. */
  squeezed?: number;
  updatedAt?: string;
}) {
  const changed = updatedAt ? changedAgo(updatedAt) : null;
  const parts = changed ? [...facts, changed] : facts;

  return (
    <Card size="sm" className="h-full">
      <CardContent className="flex flex-1 gap-3">
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <Link
            to={PREVIEWED[kind]}
            params={{ name: id }}
            className="flex min-w-0 items-start gap-3 transition-opacity hover:opacity-80"
          >
            <Icon
              className="mt-0.5 size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <span className={cn(TEXT_NAME, "min-w-0 truncate")} title={title}>
              {title}
            </span>
          </Link>
          <p
            className={cn(
              TEXT_LABEL,
              "flex min-w-0 gap-x-1.5 ps-7 font-normal whitespace-nowrap"
            )}
          >
            {parts.map((part, index) => (
              <span
                key={index}
                className={cn(
                  "not-first:before:me-1.5 not-first:before:content-['·']",
                  index === squeezed ? "min-w-0 truncate" : "shrink-0"
                )}
              >
                {part}
              </span>
            ))}
          </p>
        </div>
        <div className="-my-1 flex shrink-0 flex-col items-center justify-between gap-1">
          <CatalogueMenu kind={kind} id={id} name={name} title={title} />
          <Link
            to={PREVIEWED[kind]}
            params={{ name: id }}
            aria-label={`Open ${title}`}
            className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <ChevronRight className="size-4" aria-hidden />
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}

function CatalogueMenu({
  kind,
  id,
  name,
  title,
}: {
  kind: PreviewedKind;
  id: string;
  name: string;
  title: string;
}) {
  const [removing, setRemoving] = useState(false);
  const navigate = useNavigate();

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground"
              aria-label={`More for ${title}`}
              icon={<Ellipsis />}
            />
          }
        />
        <DropdownMenuContent align="end" className="w-48">
          <DropdownMenuItem
            onClick={() =>
              void navigate({ to: PREVIEWED[kind], params: { name: id } })
            }
          >
            <Eye />
            Open
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() =>
              void navigate({
                to: "/portal/custom/edit/$kind/$name",
                params: { kind, name: id },
              })
            }
          >
            <Pencil />
            Edit
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onClick={() => setRemoving(true)}
          >
            <Trash2 />
            Delete…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <RemoveFromCatalogue
        kind={kind}
        id={id}
        name={name}
        title={title}
        open={removing}
        onClose={() => setRemoving(false)}
      />
    </>
  );
}

function RemoveFromCatalogue({
  kind,
  id,
  name,
  title,
  open,
  onClose,
}: {
  kind: PreviewedKind;
  id: string;
  name: string;
  title: string;
  open: boolean;
  onClose: () => void;
}) {
  const definition = useRemoveDefinition();
  const dataset = useRemoveDataset();
  const [error, setError] = useState<string | null>(null);
  const noun = NOUN[kind];

  function close() {
    setError(null);
    onClose();
  }

  async function confirm() {
    setError(null);
    try {
      if (kind === "datasets") {
        await dataset.mutateAsync(name);
      } else {
        await definition.mutateAsync({ kind, id });
      }
      close();
    } catch (failure) {
      setError(refusal(failure, `The ${noun} could not be deleted.`));
    }
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      title={`Delete ${title}?`}
      description={REMOVAL_NOTE[kind]}
      confirmLabel={`Delete ${noun}`}
      destructive
      isPending={definition.isPending || dataset.isPending}
      error={error}
      onConfirm={() => void confirm()}
    />
  );
}
