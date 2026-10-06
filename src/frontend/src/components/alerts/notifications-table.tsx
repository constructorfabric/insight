import { useInfiniteQuery } from "@tanstack/react-query";

import type { Alert, AlertNotification } from "@/api/alerts-client";
import { refusal } from "@/components/custom/refusal";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { conditionText, numberText, statusText } from "@/lib/alerts/describe";
import { formatUtcInstant } from "@/lib/format";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { alertNotificationsQuery } from "@/queries/alerts";

const DAY = "d MMM yyyy";
const CLOCK = "HH:mm zzz";

type Rule = Pick<Alert, "metric" | "column">;

/**
 * Each notification once. Pages are read by offset, so one owed between two
 * reads shifts the next page and repeats a row already shown.
 */
function distinct(notifications: AlertNotification[]): AlertNotification[] {
  const seen = new Set<string>();

  return notifications.filter(({ id }) => !seen.has(id) && seen.add(id));
}

/** Every notification an alert's checks have owed, newest first. */
export function NotificationsTable({ alert }: { alert: Alert }) {
  const history = useInfiniteQuery(alertNotificationsQuery(alert.id));

  if (history.isPending) return <CenteredSpinner className="min-h-24" />;
  if (history.isError) {
    return (
      <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
        {refusal(history.error, "Couldn't read the notifications.")}
      </p>
    );
  }

  const notifications = distinct(
    history.data.pages.flatMap((page) => page.notifications)
  );
  if (notifications.length === 0) {
    return (
      <p className={cn(TEXT_BODY, "text-muted-foreground")}>
        No notifications yet
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Checked</TableHead>
            <TableHead className="text-end">Value</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Delivery</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {notifications.map((notification) => (
            <NotificationRow
              key={notification.id}
              notification={notification}
              rule={alert}
            />
          ))}
        </TableBody>
      </Table>
      {history.hasNextPage ? (
        <Button
          variant="ghost"
          size="sm"
          className="self-start"
          disabled={history.isFetchingNextPage}
          onClick={() => void history.fetchNextPage()}
        >
          {history.isFetchingNextPage ? "Reading…" : "Show older"}
        </Button>
      ) : null}
    </div>
  );
}

function NotificationRow({
  notification,
  rule,
}: {
  notification: AlertNotification;
  rule: Rule;
}) {
  return (
    <TableRow className="align-top">
      <TableCell>
        <span className={cn(TEXT_BODY, "block whitespace-nowrap")}>
          {formatUtcInstant(notification.evaluated_at, DAY)}
        </span>
        <span className={cn(TEXT_BODY, "block whitespace-nowrap")}>
          {formatUtcInstant(notification.evaluated_at, CLOCK)}
        </span>
      </TableCell>
      <TableCell className="text-end tabular-nums">
        <Reading notification={notification} rule={rule} />
      </TableCell>
      <TableCell>
        <Badge
          variant={notification.status === "sent" ? "secondary" : "outline"}
        >
          {statusText(notification.status)}
        </Badge>
      </TableCell>
      <TableCell className="w-1/2 max-w-0">
        <Delivery notification={notification} />
      </TableCell>
    </TableRow>
  );
}

/** The value a check found, against the condition it was held to then. */
function Reading({
  notification,
  rule,
}: {
  notification: AlertNotification;
  rule: Rule;
}) {
  const owedElsewhere =
    notification.metric !== rule.metric || notification.column !== rule.column;

  return (
    <span className="flex flex-col gap-0.5">
      <span className={TEXT_BODY}>{numberText(notification.value)}</span>
      <span className={cn(TEXT_LABEL, "whitespace-nowrap")}>
        {conditionText(notification.operator, notification.threshold)}
      </span>
      {owedElsewhere ? (
        <span className={cn(TEXT_LABEL, "break-words font-mono")}>
          {notification.metric} · {notification.column}
        </span>
      ) : null}
    </span>
  );
}

/** What the provider said, or what it was last refused with. */
function Delivery({ notification }: { notification: AlertNotification }) {
  const tries =
    notification.attempts === 1
      ? "1 attempt"
      : `${notification.attempts} attempts`;

  return (
    <span className="flex flex-col gap-0.5">
      <span className={TEXT_LABEL}>
        {tries} to {notification.destination}
        {notification.provider_receipt
          ? ` · receipt ${notification.provider_receipt}`
          : ""}
      </span>
      {notification.last_error ? (
        <span
          className={cn(
            TEXT_BODY,
            "break-words",
            notification.status === "failed"
              ? "text-destructive"
              : "text-muted-foreground"
          )}
        >
          {notification.last_error}
        </span>
      ) : null}
    </span>
  );
}
