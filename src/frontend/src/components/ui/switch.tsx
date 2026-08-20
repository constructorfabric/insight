import { Switch as KitSwitch, type SwitchProps } from "@gears-frontx/ui-kit";

function Switch({ size = "default", ...props }: SwitchProps) {
  return <KitSwitch data-slot="switch" data-size={size} size={size} {...props} />;
}

export { Switch };
