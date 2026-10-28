import { Link, useNavigate } from "@tanstack/react-router";
import { Pencil } from "lucide-react";

import type { DashboardAccess } from "@/api/custom-client";
import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { Button } from "@/components/ui/button";
import { dashboardAllows } from "@/lib/custom/dashboard-access";

export function DashboardPageActions({
  id,
  name,
  title,
  access,
  ownerPersonId,
  carries,
}: {
  id: string;
  name: string;
  title: string;
  access: DashboardAccess;
  ownerPersonId: string | null;
  carries: boolean;
}) {
  const navigate = useNavigate();
  const allows = dashboardAllows(access);

  return (
    <span className="flex shrink-0 items-center gap-1">
      {allows.edit ? (
        <Button
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
      ) : null}
      <DashboardMenu
        id={id}
        name={name}
        title={title}
        access={access}
        share={allows.share && carries ? { ownerPersonId } : undefined}
        onDuplicated={(copy) =>
          void navigate({ to: "/portal/custom/$name", params: { name: copy } })
        }
        onDeleted={() => void navigate({ to: "/portal/custom" })}
      />
    </span>
  );
}
