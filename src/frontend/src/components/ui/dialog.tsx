import {
  DialogClose as KitDialogClose,
  DialogContent as KitDialogContent,
  type DialogCloseProps,
  type DialogContentProps,
} from "@gears-frontx/ui-kit";

import { cn } from "@/lib/utils";

function DialogClose(props: DialogCloseProps) {
  return <KitDialogClose data-slot="dialog-close" {...props} />;
}

// The kit's popup is a grid; its items still default to min-width:auto, so an
// unbreakable value pushes out through the side without this.
function DialogContent({ className, ...props }: DialogContentProps) {
  return <KitDialogContent className={cn("[&>*]:min-w-0", className)} {...props} />;
}

const DIALOG_FILL_WINDOW = "h-[96vh] w-[98vw] max-w-[98vw]";

export {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@gears-frontx/ui-kit";
export { DIALOG_FILL_WINDOW, DialogClose, DialogContent };
