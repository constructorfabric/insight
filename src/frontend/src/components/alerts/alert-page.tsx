import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useId, type ReactNode } from "react";

import type { Alert, AlertState } from "@/api/alerts-client";
import { AlertSwitch } from "@/components/alerts/alert-switch";
import { NotificationsTable } from "@/components/alerts/notifications-table";
import { ConfirmRemove } from "@/components/custom/confirm-remove";
import { refusal } from "@/components/custom/refusal";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { conditionText, numberText, reasonText } from "@/lib/alerts/describe";
import { intervalLabel } from "@/lib/alerts/interval";
import { rangeLabel } from "@/lib/custom/time-range";
import { formatUtcAge, formatUtcInstant } from "@/lib/format";
import {
  TEXT_BODY,
  TEXT_HEADING,
  TEXT_LABEL,
  TEXT_TITLE,
} from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { alertQuery, useDeleteAlert } from "@/queries/alerts";

const WHEN = "d MMM yyyy, HH:mm zzz";

const OFF_HINT = "Turning it off withdraws notifications not yet sent.";

function BackToAlerts() {
  return (
    <Link
      to="/portal/custom/alerts"
      className={cn(
        TEXT_BODY,
        "self-start underline decoration-dotted underline-offset-4"
      )}
    >
      Back to alerts
    </Link>
  );
}

export function AlertPage({ id }: { id: string }) {
  const alert = useQuery(alertQuery(id));

  if (alert.isPending) return <CenteredSpinner className="min-h-40" />;
  if (alert.isError) {
    return (
      <div className="flex flex-col gap-2 p-4 md:p-6">
        <h1 className={TEXT_TITLE}>Alert</h1>
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(alert.error, "Alert not found.")}
        </p>
        <BackToAlerts />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <Header alert={alert.data} />
      <div className="grid gap-4 @3xl:grid-cols-2">
        <Rule alert={alert.data} />
        <LatestCheck state={alert.data.state} enabled={alert.data.enabled} />
      </div>
      <Card>
        <CardHeader>
          <CardTitle className={TEXT_HEADING}>Notifications</CardTitle>
        </CardHeader>
        <CardContent>
          <NotificationsTable alert={alert.data} />
        </CardContent>
      </Card>
    </div>
  );
}

function Header({ alert }: { alert: Alert }) {
  const remove = useDeleteAlert();
  const navigate = useNavigate();
  const hintId = useId();

  return (
    <header className="flex flex-wrap items-start gap-3">
      <div className="flex min-w-0 grow flex-col gap-1">
        <h1 className={cn(TEXT_TITLE, "break-words")}>{alert.name}</h1>
        <BackToAlerts />
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1">
        <div className="flex flex-wrap items-start justify-end gap-2">
          <AlertSwitch
            id={alert.id}
            name={alert.name}
            enabled={alert.enabled}
            revision={alert.revision}
            describedBy={alert.enabled ? hintId : undefined}
          />
          <Button
            variant="outline"
            size="sm"
            nativeButton={false}
            render={
              <Link
                to="/portal/custom/alerts/$id/edit"
                params={{ id: alert.id }}
              />
            }
          >
            Edit
          </Button>
          <ConfirmRemove
            ask={(open) => (
              <Button variant="ghost" size="sm" onClick={open}>
                Delete
              </Button>
            )}
            confirm="Delete it, with its notifications"
            pending={remove.isPending}
            error={remove.error}
            onRemove={() =>
              remove.mutate(alert.id, {
                onSuccess: () => void navigate({ to: "/portal/custom/alerts" }),
              })
            }
            onKeep={() => remove.reset()}
          />
        </div>
        {alert.enabled ? (
          <p id={hintId} className={cn(TEXT_LABEL, "max-w-64 text-end")}>
            {OFF_HINT}
          </p>
        ) : null}
      </div>
    </header>
  );
}

function Fact({ term, children }: { term: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className={TEXT_LABEL}>{term}</dt>
      <dd className={cn(TEXT_BODY, "min-w-0 break-words")}>{children}</dd>
    </div>
  );
}

function Rule({ alert }: { alert: Alert }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>Rule</CardTitle>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-3 sm:grid-cols-2">
          <Fact term="Metric">
            <Link
              to="/portal/custom/metrics/$name"
              params={{ name: alert.metric }}
              className="font-mono underline-offset-4 hover:underline"
            >
              {alert.metric}
            </Link>
          </Fact>
          <Fact term="Column">
            <code className="font-mono">{alert.column}</code>
          </Fact>
          <Fact term="Condition">
            {conditionText(alert.operator, alert.threshold)}
          </Fact>
          <Fact term="Window">
            {alert.range ? rangeLabel(alert.range) : "All time"}
          </Fact>
          <Fact term="Check every">{intervalLabel(alert.interval_secs)}</Fact>
          <Fact term="Destination">{alert.destination}</Fact>
        </dl>
      </CardContent>
    </Card>
  );
}

function LatestCheck({
  state,
  enabled,
}: {
  state: AlertState;
  enabled: boolean;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>Latest check</CardTitle>
      </CardHeader>
      <CardContent>
        {!state.last_evaluated_at ? (
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            {enabled ? "Not checked yet" : "Disabled"}
          </p>
        ) : (
          <dl className="grid gap-3 sm:grid-cols-2">
            <Fact term="Outcome">
              <Outcome state={state} />
            </Fact>
            <Fact term="When">
              {formatUtcInstant(state.last_evaluated_at, WHEN)}
              <span className={cn(TEXT_LABEL, "block")}>
                {formatUtcAge(state.last_evaluated_at)}
              </span>
            </Fact>
            {state.last_value !== undefined ? (
              <Fact term="Value">
                <span className="tabular-nums">
                  {numberText(state.last_value)}
                </span>
              </Fact>
            ) : null}
            {state.breached_since ? (
              <Fact term="Met since">
                {formatUtcInstant(state.breached_since, WHEN)}
              </Fact>
            ) : null}
            {state.last_reason ? (
              <Fact term="Reason">{reasonText(state.last_reason)}</Fact>
            ) : null}
          </dl>
        )}
      </CardContent>
    </Card>
  );
}

function Outcome({ state }: { state: AlertState }) {
  switch (state.last_outcome) {
    case "breach":
      return <Badge variant="info">Condition met</Badge>;
    case "no_breach":
      return <Badge variant="secondary">Condition not met</Badge>;
    case "unknown":
      return <Badge variant="outline">Unknown</Badge>;
    case undefined:
      return null;
  }
}
