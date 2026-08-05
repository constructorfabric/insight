import { Card, CardContent } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";

type FullScreenLoadingProps = {
  message?: string;
};

export function FullScreenLoading({
  message,
}: FullScreenLoadingProps): React.ReactElement {
  return (
    <div className="flex min-h-svh w-full items-center justify-center p-6">
      <Card className="w-full max-w-sm">
        <CardContent className="flex flex-col items-center gap-3 text-center">
          <Spinner className="size-6" />
          {message ? (
            <p className="text-sm text-muted-foreground">{message}</p>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
