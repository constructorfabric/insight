import { Link, useNavigate } from "@tanstack/react-router";
import { Pencil } from "lucide-react";

import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { Button } from "@/components/ui/button";

export function DashboardPageActions({
  id,
  name,
  title,
}: {
  id: string;
  name: string;
  title: string;
}) {
  const navigate = useNavigate();

  return (
    <span className="flex shrink-0 items-center gap-1">
      <Button
        variant="outline"
        size="sm"
        icon={<Pencil />}
        aria-label={`Edit ${name}`}
        nativeButton={false}
        render={
          <Link
            to="/portal/custom/edit/$kind/$name"
            params={{ kind: "dashboards", name: id }}
          />
        }
      >
        Edit
      </Button>
      <DashboardMenu
        id={id}
        name={name}
        title={title}
        filing={false}
        onDuplicated={(copy) =>
          void navigate({ to: "/portal/custom/$name", params: { name: copy } })
        }
        onDeleted={() => void navigate({ to: "/portal/custom" })}
      />
    </span>
  );
}
