/**
 * Feedback over the period the rest of the usage surface is showing. Its own
 * query, so a failed read here leaves the usage numbers standing.
 */
import type { FeedbackEntry, FeedbackRange } from "@/api/feedback-client";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { VirtualTable } from "@/components/portal/usage-table";
import { formatUtcClock } from "@/lib/format";
import { screenLabel } from "@/lib/portal/screen-label";
import { visitorLabel } from "@/lib/portal/visitor-label";
import { TEXT_NAME } from "@/lib/type-scale";
import { useFeedbackList } from "@/queries/feedback";

const CATEGORY_LABEL: Record<string, string> = {
  bug: "Broken",
  idea: "Idea",
  confusing: "Confusing",
  other: "Other",
};

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
        { header: "Person", width: 12, cell: (row) => <SenderName row={row} /> },
        {
          header: "About",
          width: 7,
          cell: (row) => CATEGORY_LABEL[row.category] ?? row.category,
        },
        { header: "Feedback", cell: (row) => <Message text={row.message} /> },
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

function Message({ text }: { text: string }) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger render={<span className="truncate" />}>{text}</TooltipTrigger>
        <TooltipContent className="max-w-sm text-xs leading-relaxed">
          {text}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

function SenderName({ row }: { row: FeedbackEntry }) {
  const { label, detail } = visitorLabel(row);

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger render={<span className="truncate" />}>{label}</TooltipTrigger>
        <TooltipContent className="font-mono text-xs">{detail}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
