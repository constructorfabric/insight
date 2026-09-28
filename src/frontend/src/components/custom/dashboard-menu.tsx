import { Ellipsis } from "lucide-react";
import { useState } from "react";

import {
  MoveToFolderItems,
  NewFolderAndMove,
} from "@/components/custom/move-to-folder";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export function DashboardMenu({ name }: { name: string }) {
  const [open, setOpen] = useState(false);
  const [naming, setNaming] = useState(false);

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
          <MoveToFolderItems
            name={name}
            open={open}
            onNewFolder={() => setNaming(true)}
          />
        </DropdownMenuContent>
      </DropdownMenu>
      <NewFolderAndMove
        name={name}
        open={naming}
        onClose={() => setNaming(false)}
      />
    </>
  );
}
