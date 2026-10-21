import {
  Copy,
  Ellipsis,
  Pin,
  PinOff,
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
import { EditTags } from "@/components/custom/edit-tags";
import {
  MoveToFolderItems,
  NewFolderAndMove,
} from "@/components/custom/move-to-folder";
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

type Asked = "rename" | "duplicate" | "delete" | "folder" | "tags";

export function DashboardMenu({
  id,
  name,
  title,
  access,
  onDuplicated,
  onDeleted,
}: {
  id: string;
  name: string;
  title?: string;
  access: DashboardAccess;
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
          {allows.change ? (
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
          <DropdownMenuSeparator />
          <MoveToFolderItems
            id={id}
            access={access}
            open={open}
            onNewFolder={() => setAsked("folder")}
          />
          {allows.change ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => setAsked("tags")}>
                <Tags />
                Edit tags…
              </DropdownMenuItem>
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
      <NewFolderAndMove
        id={id}
        name={name}
        open={asked === "folder"}
        onClose={done}
      />
      {allows.change ? (
        <>
          <RenameDashboard
            id={id}
            name={name}
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
          <DeleteDashboard
            id={id}
            title={title || name}
            open={asked === "delete"}
            onClose={done}
            onDeleted={onDeleted}
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
