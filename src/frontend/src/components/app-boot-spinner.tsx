import { Spinner } from "@gears-frontx/ui-kit";

export function AppBootSpinner(): React.ReactElement {
  return (
    <div className="flex min-h-svh w-full items-center justify-center">
      <Spinner className="size-12 text-muted-foreground" />
    </div>
  );
}
