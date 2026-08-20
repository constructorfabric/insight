import { Separator as KitSeparator, type SeparatorProps } from "@gears-frontx/ui-kit";

function Separator({ orientation = "horizontal", ...props }: SeparatorProps) {
  return <KitSeparator data-slot="separator" orientation={orientation} {...props} />;
}

export { Separator };
