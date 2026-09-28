import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

const SHOWN_TAGS = 3;

export function DashboardTags({
  tags,
  className,
}: {
  tags: string[];
  className?: string;
}) {
  if (tags.length === 0) return null;

  const shown = tags.slice(0, SHOWN_TAGS);
  const rest = tags.slice(SHOWN_TAGS);

  return (
    <ul aria-label="Tags" className={cn("flex flex-wrap gap-1", className)}>
      {shown.map((tag) => (
        <li key={tag} className="max-w-full min-w-0">
          <Badge variant="secondary" className="max-w-full">
            {tag}
          </Badge>
        </li>
      ))}
      {rest.length > 0 ? (
        <li>
          <Badge variant="outline" title={rest.join(", ")}>
            +{rest.length}
          </Badge>
        </li>
      ) : null}
    </ul>
  );
}
