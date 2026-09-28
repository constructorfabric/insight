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

type Asked = "rename" | "duplicate" | "delete" | "folder" | "tags";

export function DashboardMenu({
  name,
  title,
  filing = true,
  onRenamed,
  onDuplicated,
  onDeleted,
}: {
  name: string;
  title?: string;
  filing?: boolean;
  onRenamed?: (to: string) => void;
  onDuplicated?: (to: string) => void;
  onDeleted?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [asked, setAsked] = useState<Asked | null>(null);
  const pin = usePinToggle(name, open);

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
          <DropdownMenuItem onClick={() => setAsked("rename")}>
            <TextCursorInput />
            Rename…
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setAsked("duplicate")}>
            <Copy />
            Duplicate…
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          {filing ? (
            <>
              <MoveToFolderItems
                name={name}
                open={open}
                onNewFolder={() => setAsked("folder")}
              />
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => setAsked("tags")}>
                <Tags />
                Edit tags…
              </DropdownMenuItem>
              <DropdownMenuSeparator />
            </>
          ) : null}
          <DropdownMenuItem
            variant="destructive"
            onClick={() => setAsked("delete")}
          >
            <Trash2 />
            Delete…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <RenameDashboard
        name={name}
        open={asked === "rename"}
        onClose={done}
        onRenamed={onRenamed}
      />
      <DuplicateDashboard
        name={name}
        open={asked === "duplicate"}
        onClose={done}
        onDuplicated={onDuplicated}
      />
      <DeleteDashboard
        name={name}
        title={title || name}
        open={asked === "delete"}
        onClose={done}
        onDeleted={onDeleted}
      />
      {filing ? (
        <>
          <NewFolderAndMove
            name={name}
            open={asked === "folder"}
            onClose={done}
          />
          <EditTags name={name} open={asked === "tags"} onClose={done} />
        </>
      ) : null}
    </>
  );
}
