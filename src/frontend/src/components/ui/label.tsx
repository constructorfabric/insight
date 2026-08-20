import { Label as KitLabel, type LabelProps } from "@gears-frontx/ui-kit";

function Label(props: LabelProps) {
  return <KitLabel data-slot="label" {...props} />;
}

export { Label };
