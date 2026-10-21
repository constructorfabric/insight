import { Link, useNavigate } from "@tanstack/react-router";
import { Pencil, Share2 } from "lucide-react";
import { useState } from "react";

import type { DashboardAccess } from "@/api/custom-client";
import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { ShareDashboard } from "@/components/custom/share-dashboard";
import { Button } from "@/components/ui/button";
import { dashboardAllows } from "@/lib/custom/dashboard-access";

export function DashboardPageActions({
  id,
  name,
  title,
  access,
  ownerPersonId,
}: {
  id: string;
  name: string;
  title: string;
  access: DashboardAccess;
  ownerPersonId: string | null;
}) {
  const navigate = useNavigate();
  const [sharing, setSharing] = useState(false);
  const allows = dashboardAllows(access);

  return (
    <span className="flex shrink-0 items-center gap-1">
      {allows.share && ownerPersonId ? (
        <>
          <Button
            variant="outline"
            size="sm"
            icon={<Share2 />}
            aria-label={`Share ${name}`}
            onClick={() => setSharing(true)}
          >
            Share
          </Button>
          <ShareDashboard
            id={id}
            title={title || name}
            ownerPersonId={ownerPersonId}
            open={sharing}
            onClose={() => setSharing(false)}
          />
        </>
      ) : null}
      {allows.change ? (
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
      ) : null}
      <DashboardMenu
        id={id}
        name={name}
        title={title}
        access={access}
        onDuplicated={(copy) =>
          void navigate({ to: "/portal/custom/$name", params: { name: copy } })
        }
        onDeleted={() => void navigate({ to: "/portal/custom" })}
      />
    </span>
  );
}
