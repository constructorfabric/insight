import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";

import type { EditableKind } from "@/api/custom-client";
import { NewDashboardButton } from "@/components/custom/create-dashboard-dialog";
import { Button } from "@/components/ui/button";
import { PREVIEWED, type PreviewedKind } from "@/lib/custom/catalogue-paths";

export function EditLink({
  kind,
  id,
  name,
}: {
  kind: EditableKind;
  id: string;
  name: string;
}) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className="text-muted-foreground"
      aria-label={`Edit ${name}`}
      nativeButton={false}
      render={
        <Link
          to="/portal/custom/edit/$kind/$name"
          params={{ kind, name: id }}
        />
      }
    >
      Edit
    </Button>
  );
}

export function NewLink({ kind, noun }: { kind: EditableKind; noun: string }) {
  if (kind === "dashboards") return <NewDashboardButton />;

  return (
    <Button
      size="sm"
      icon={<Plus />}
      nativeButton={false}
      render={<Link to="/portal/custom/new/$kind" params={{ kind }} />}
    >
      New {noun}
    </Button>
  );
}

export function PreviewLink({
  kind,
  id,
  name,
}: {
  kind: PreviewedKind;
  id: string;
  name: string;
}) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className="text-muted-foreground"
      aria-label={`Preview ${name}`}
      nativeButton={false}
      render={<Link to={PREVIEWED[kind]} params={{ name: id }} />}
    >
      Preview
    </Button>
  );
}
