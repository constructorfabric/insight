import { Link, useNavigate } from "@tanstack/react-router";
import { useInfiniteQuery } from "@tanstack/react-query";
import { Bell, Ellipsis, Eye, Pencil, Plus, Trash2 } from "lucide-react";
import { useState } from "react";

import type { AlertSummary } from "@/api/alerts-client";
import { AlertSwitch } from "@/components/alerts/alert-switch";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { CardFrame } from "@/components/custom/catalogue-card";
import { DefinitionList } from "@/components/custom/definition-list";
import { MetricName } from "@/components/custom/metric-name";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { alertPagesQuery, useDeleteAlert } from "@/queries/alerts";

/** Long enough that a word is typed before the service hears about it. */
const SEARCH_DEBOUNCE_MS = 400;

export function AlertsList() {
  const [needle, setNeedle] = useState("");
  const searching = useDebouncedValue(needle, SEARCH_DEBOUNCE_MS).trim();
  const pages = useInfiniteQuery(alertPagesQuery(searching));
  const alerts = pages.data?.pages.flatMap((page) => page.alerts);

  return (
    <DefinitionList
      data-testid="page-custom-alerts"
      title="Alerts"
      items={alerts}
      isLoading={pages.isPending}
      isError={pages.isError}
      errorLabel={
        pages.isError
          ? refusal(pages.error, "Couldn't load the alerts.")
          : undefined
      }
      onRetry={() => void pages.refetch()}
      search={{ label: "Search alerts", value: needle, onChange: setNeedle }}
      paging={{
        total: pages.data?.pages.at(-1)?.total,
        hasMore: pages.hasNextPage,
        isFetchingMore: pages.isFetchingNextPage,
        onMore: () => void pages.fetchNextPage(),
        searching: searching !== "",
      }}
      create={
        <Button
          size="sm"
          icon={<Plus />}
          nativeButton={false}
          render={<Link to="/portal/custom/alerts/new" />}
        >
          New alert
        </Button>
      }
      empty={{ icon: Bell, title: "No alerts yet" }}
      renderRow={(alert) => <AlertCard alert={alert} />}
    />
  );
}

function AlertCard({ alert }: { alert: AlertSummary }) {
  return (
    <CardFrame
      title={alert.name}
      open={{ to: "/portal/custom/alerts/$id", params: { id: alert.id } }}
      icon={Bell}
      facts={[<MetricName key="metric" id={alert.metric} />]}
      control={
        <AlertSwitch id={alert.id} name={alert.name} enabled={alert.enabled} />
      }
      menu={<AlertMenu alert={alert} />}
    />
  );
}

function AlertMenu({ alert }: { alert: AlertSummary }) {
  const [removing, setRemoving] = useState(false);
  const navigate = useNavigate();
  const remove = useDeleteAlert();
  const params = { id: alert.id };

  function close() {
    remove.reset();
    setRemoving(false);
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground"
              aria-label={`More for ${alert.name}`}
              icon={<Ellipsis />}
            />
          }
        />
        <DropdownMenuContent align="end" className="w-48">
          <DropdownMenuItem
            onClick={() =>
              void navigate({ to: "/portal/custom/alerts/$id", params })
            }
          >
            <Eye />
            Open
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() =>
              void navigate({ to: "/portal/custom/alerts/$id/edit", params })
            }
          >
            <Pencil />
            Edit
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onClick={() => setRemoving(true)}
          >
            <Trash2 />
            Delete…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ConfirmDialog
        open={removing}
        onOpenChange={(next) => {
          if (!next) close();
        }}
        title={`Delete ${alert.name}?`}
        description="Its notifications are deleted with it. This can't be undone."
        confirmLabel="Delete alert"
        destructive
        isPending={remove.isPending}
        error={
          remove.isError
            ? refusal(remove.error, "The alert could not be deleted.")
            : null
        }
        onConfirm={() => {
          remove.reset();
          remove.mutate(alert.id, { onSuccess: close });
        }}
      />
    </>
  );
}
