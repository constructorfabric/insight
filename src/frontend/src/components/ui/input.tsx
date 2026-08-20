import { Input as KitInput, type InputProps } from "@gears-frontx/ui-kit";

function Input(props: InputProps) {
  return <KitInput data-slot="input" {...props} />;
}

export { Input };
