import {
  CalendarClock,
  Copy,
  Ellipsis,
  FileDown,
  FolderPlus,
  Pin,
  PinOff,
  Share2,
  Tags,
  TextCursorInput,
  Trash2,
} from "lucide-react";
import { useState, type ReactNode } from "react";

import type { DashboardAccess } from "@/api/custom-client";
import {
  DeleteDashboard,
  DuplicateDashboard,
  RenameDashboard,
} from "@/components/custom/dashboard-dialogs";
import { AddToFolder } from "@/components/custom/dashboard-folders";
import { EditTags } from "@/components/custom/edit-tags";
import { ShareDialog } from "@/components/custom/share-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { usePinToggle } from "@/hooks/use-pin-toggle";
import { dashboardAllows } from "@/lib/custom/dashboard-access";

type Asked = "rename" | "duplicate" | "delete" | "folder" | "tags" | "share";

export function DashboardMenu({
  id,
  name,
  title,
  access,
  share,
  planned = false,
  onDuplicated,
  onDeleted,
}: {
  id: string;
  name: string;
  title?: string;
  access: DashboardAccess;
  share?: { ownerPersonId: string | null };
  /** Whether to list the actions not built yet, as an open dashboard does. */
  planned?: boolean;
  onDuplicated?: (id: string) => void;
  onDeleted?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [asked, setAsked] = useState<Asked | null>(null);
  const pin = usePinToggle(id, open);
  const allows = dashboardAllows(access);

  const done = () => setAsked(null);

  return (
    <>
      <DropdownMenu open={open} onOpenChange={setOpen}>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground"
              aria-label={`More for ${name}`}
              icon={<Ellipsis />}
            />
          }
        />
        <DropdownMenuContent align="end" className={planned ? "w-64" : "w-56"}>
          {allows.edit ? (
            <DropdownMenuItem onClick={() => setAsked("duplicate")}>
              <Copy />
              Duplicate…
            </DropdownMenuItem>
          ) : null}
          {share ? (
            <DropdownMenuItem onClick={() => setAsked("share")}>
              <Share2 />
              Share…
            </DropdownMenuItem>
          ) : null}
          {planned ? (
            <>
              <PlannedItem icon={<FileDown />} label="Export snapshot (PDF)" />
              <PlannedItem icon={<CalendarClock />} label="Schedule report…" />
            </>
          ) : null}
          {allows.edit ? (
            <DropdownMenuItem onClick={() => setAsked("rename")}>
              <TextCursorInput />
              Rename…
            </DropdownMenuItem>
          ) : null}
          <DropdownMenuItem disabled={!pin.ready} onClick={pin.toggle}>
            {pin.pinned ? <PinOff /> : <Pin />}
            {pin.pinned ? "Unpin" : "Pin"}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setAsked("folder")}>
            <FolderPlus />
            Add to folder…
          </DropdownMenuItem>
          {allows.edit ? (
            <DropdownMenuItem onClick={() => setAsked("tags")}>
              <Tags />
              Edit tags…
            </DropdownMenuItem>
          ) : null}
          {allows.remove ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                variant="destructive"
                onClick={() => setAsked("delete")}
              >
                <Trash2 />
                Delete…
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
      {share ? (
        <ShareDialog
          target={{ kind: "dashboards", id }}
          title={title || name}
          ownerPersonId={share.ownerPersonId}
          open={asked === "share"}
          onClose={done}
        />
      ) : null}
      <AddToFolder
        id={id}
        name={name}
        access={access}
        open={asked === "folder"}
        onClose={done}
      />
      {allows.remove ? (
        <DeleteDashboard
          id={id}
          title={title || name}
          open={asked === "delete"}
          onClose={done}
          onDeleted={onDeleted}
        />
      ) : null}
      {allows.edit ? (
        <>
          <RenameDashboard
            id={id}
            name={title || name}
            open={asked === "rename"}
            onClose={done}
          />
          <DuplicateDashboard
            id={id}
            name={name}
            open={asked === "duplicate"}
            onClose={done}
            onDuplicated={onDuplicated}
          />
          <EditTags
            id={id}
            name={name}
            open={asked === "tags"}
            onClose={done}
          />
        </>
      ) : null}
    </>
  );
}

function PlannedItem({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <DropdownMenuItem disabled>
      {icon}
      {label}
      <span className="ms-auto text-xs text-muted-foreground">Planned</span>
    </DropdownMenuItem>
  );
}
