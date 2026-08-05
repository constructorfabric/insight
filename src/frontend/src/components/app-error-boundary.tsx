import { QueryErrorResetBoundary } from "@tanstack/react-query";
import { type ReactNode } from "react";
import { ErrorBoundary } from "react-error-boundary";

import { ErrorFallback } from "./error-fallback";

export function AppErrorBoundary({
  children,
}: {
  children: ReactNode;
}): React.ReactElement {
  return (
    <QueryErrorResetBoundary>
      {({ reset }) => (
        <ErrorBoundary FallbackComponent={ErrorFallback} onReset={reset}>
          {children}
        </ErrorBoundary>
      )}
    </QueryErrorResetBoundary>
  );
}
