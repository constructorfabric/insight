/**
 * Feedback over the period the rest of the usage surface is showing. Its own
 * query, so a failed read here leaves the usage numbers standing.
 */
import type { FeedbackRange } from "@/api/feedback-client";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import {
  PersonName,
  TruncatedCell,
  VirtualTable,
} from "@/components/portal/usage-table";
import { formatUtcClock } from "@/lib/format";
import { screenLabel } from "@/lib/portal/screen-label";
import { TEXT_NAME } from "@/lib/type-scale";
import { useFeedbackList } from "@/queries/feedback";

export function FeedbackTable({ range }: { range: FeedbackRange }) {
  const feedback = useFeedbackList(range);

  return (
    <section className="flex flex-col gap-2">
      <h3 className={TEXT_NAME}>What people told us</h3>
      <Body query={feedback} />
    </section>
  );
}

function Body({ query }: { query: ReturnType<typeof useFeedbackList> }) {
  if (query.isPending) return <CenteredSpinner />;
  if (query.isError || !query.data) {
    return (
      <ComingSoon variant="row" state="empty" label="Feedback could not be loaded" />
    );
  }
  if (query.data.items.length === 0) {
    return <ComingSoon variant="row" state="empty" label="No feedback in this period" />;
  }

  return (
    <VirtualTable
      label="What people told us"
      rows={query.data.items}
      rowKey={(row) => row.feedback_id}
      columns={[
        {
          header: "When (UTC)",
          width: 11,
          cell: (row) => formatUtcClock(row.ts, "d MMM HH:mm"),
        },
        { header: "Person", width: 12, cell: (row) => <PersonName row={row} /> },
        {
          header: "Feedback",
          cell: (row) => (
            <TruncatedCell
              detail={row.message}
              detailClassName="max-w-sm text-xs leading-relaxed"
            >
              {row.message}
            </TruncatedCell>
          ),
        },
        {
          header: "Screen",
          width: 10,
          cell: (row) => (
            <span className="truncate text-xs text-muted-foreground">
              {row.path ? screenLabel(row.path) : "—"}
            </span>
          ),
        },
      ]}
    />
  );
}
