import { Ellipsis, Pin, PinOff, Tags } from "lucide-react";
import { useState } from "react";

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

export function DashboardMenu({ name }: { name: string }) {
  const [open, setOpen] = useState(false);
  const [naming, setNaming] = useState(false);
  const [tagging, setTagging] = useState(false);
  const pin = usePinToggle(name, open);

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
          <DropdownMenuSeparator />
          <MoveToFolderItems
            name={name}
            open={open}
            onNewFolder={() => setNaming(true)}
          />
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => setTagging(true)}>
            <Tags />
            Edit tags…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <NewFolderAndMove
        name={name}
        open={naming}
        onClose={() => setNaming(false)}
      />
      <EditTags name={name} open={tagging} onClose={() => setTagging(false)} />
    </>
  );
}
