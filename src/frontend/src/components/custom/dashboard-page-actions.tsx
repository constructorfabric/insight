import { Link, useNavigate } from "@tanstack/react-router";
import { Pencil } from "lucide-react";

import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { Button } from "@/components/ui/button";

export function DashboardPageActions({
  name,
  title,
}: {
  name: string;
  title: string;
}) {
  const navigate = useNavigate();

  const land = (to: string) =>
    void navigate({ to: "/portal/custom/$name", params: { name: to } });

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
            params={{ kind: "dashboards", name }}
          />
        }
      >
        Edit
      </Button>
      <DashboardMenu
        name={name}
        title={title}
        filing={false}
        onRenamed={land}
        onDuplicated={land}
        onDeleted={() => void navigate({ to: "/portal/custom" })}
      />
    </span>
  );
}
