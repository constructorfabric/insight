import { Checkbox as KitCheckbox, type CheckboxProps } from "@gears-frontx/ui-kit";

function Checkbox(props: CheckboxProps) {
  return <KitCheckbox data-slot="checkbox" {...props} />;
}

export { Checkbox };
