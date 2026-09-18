import { Link } from "@tanstack/react-router";
import { Braces } from "lucide-react";

import type { EditableKind } from "@/api/custom-client";
import { Button } from "@/components/ui/button";

export function EditLink({ kind, name }: { kind: EditableKind; name: string }) {
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      className="text-muted-foreground"
      aria-label={`Edit ${name}`}
      nativeButton={false}
      render={
        <Link to="/portal/custom/edit/$kind/$name" params={{ kind, name }} />
      }
    >
      <Braces />
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
