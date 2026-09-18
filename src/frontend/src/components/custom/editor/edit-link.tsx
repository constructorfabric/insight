import { Link } from "@tanstack/react-router";

import type { EditableKind } from "@/api/custom-client";
import { Button } from "@/components/ui/button";

export function EditLink({ kind, name }: { kind: EditableKind; name: string }) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className="text-muted-foreground"
      aria-label={`Edit ${name}`}
      nativeButton={false}
      render={
        <Link to="/portal/custom/edit/$kind/$name" params={{ kind, name }} />
      }
    >
      Edit
    </Button>
  );
}

export function NewLink({ kind, noun }: { kind: EditableKind; noun: string }) {
  return (
    <Button
      variant="outline"
      size="sm"
      nativeButton={false}
      render={<Link to="/portal/custom/new/$kind" params={{ kind }} />}
    >
      New {noun}
    </Button>
  );
}

/** The kinds that have a page of their own to look at, and where it is. */
const PREVIEWED = {
  datasets: "/portal/custom/datasets/$name",
  metrics: "/portal/custom/metrics/$name",
  widgets: "/portal/custom/widgets/$name",
} as const;

export type PreviewedKind = keyof typeof PREVIEWED;

export function PreviewLink({
  kind,
  name,
}: {
  kind: PreviewedKind;
  name: string;
}) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className="text-muted-foreground"
      aria-label={`Preview ${name}`}
      nativeButton={false}
      render={<Link to={PREVIEWED[kind]} params={{ name }} />}
    >
      Preview
    </Button>
  );
}
