import { Link } from "@tanstack/react-router";
import { Braces } from "lucide-react";

import type { EditableKind } from "@/api/custom-client";
import { buttonVariants } from "@/components/ui/button";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

// The kit's Button renders its own element, so a link that looks like one is
// the anchor with the button's classes rather than a button wrapping it.
const ICON = cn(
  buttonVariants({ variant: "ghost" }),
  "inline-flex size-8 items-center justify-center rounded-md text-muted-foreground"
);

/** Opens one definition in the editor, by the name it is stored under. */
export function EditLink({ kind, name }: { kind: EditableKind; name: string }) {
  return (
    <Link
      to="/portal/custom/edit/$kind/$name"
      params={{ kind, name }}
      aria-label={`Edit ${name}`}
      className={ICON}
    >
      <Braces className="size-4" />
    </Link>
  );
}

/** Starts a definition of a kind that does not exist yet. */
export function NewLink({ kind, noun }: { kind: EditableKind; noun: string }) {
  return (
    <Link
      to="/portal/custom/new/$kind"
      params={{ kind }}
      className={cn(
        buttonVariants({ variant: "outline" }),
        TEXT_LABEL,
        "inline-flex h-8 items-center rounded-md px-3"
      )}
    >
      New {noun}
    </Link>
  );
}
