import {
  Copy,
  Ellipsis,
  FolderPlus,
  Pin,
  PinOff,
  Share2,
  Tags,
  TextCursorInput,
  Trash2,
} from "lucide-react";
import { useState } from "react";

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
  onDuplicated,
  onDeleted,
}: {
  id: string;
  name: string;
  title?: string;
  access: DashboardAccess;
  share?: { ownerPersonId: string | null };
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
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuItem disabled={!pin.ready} onClick={pin.toggle}>
            {pin.pinned ? <PinOff /> : <Pin />}
            {pin.pinned ? "Unpin" : "Pin"}
          </DropdownMenuItem>
          {share ? (
            <DropdownMenuItem onClick={() => setAsked("share")}>
              <Share2 />
              Share…
            </DropdownMenuItem>
          ) : null}
          {allows.edit ? (
            <>
              <DropdownMenuItem onClick={() => setAsked("rename")}>
                <TextCursorInput />
                Rename…
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => setAsked("duplicate")}>
                <Copy />
                Duplicate…
              </DropdownMenuItem>
            </>
          ) : null}
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
