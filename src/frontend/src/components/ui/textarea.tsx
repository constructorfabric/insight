import { Textarea as KitTextarea, type TextareaProps } from "@gears-frontx/ui-kit";

function Textarea(props: TextareaProps) {
  return <KitTextarea data-slot="textarea" {...props} />;
}

export { Textarea };
