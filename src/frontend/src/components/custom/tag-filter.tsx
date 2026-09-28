import { Tag } from "lucide-react";

import type { TagSummary } from "@/api/custom-client";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function TagFilter({
  tags,
  picked,
  onPick,
}: {
  tags: TagSummary[] | undefined;
  picked: string[];
  onPick: (next: string[]) => void;
}) {
  const toggle = (name: string) =>
    onPick(
      picked.includes(name)
        ? picked.filter((one) => one !== name)
        : [...picked, name]
    );

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button type="button" variant="outline" size="sm" icon={<Tag />}>
            {picked.length > 0 ? `Tags · ${picked.length}` : "Tags"}
          </Button>
        }
      />
      <DropdownMenuContent align="start" className="w-56">
        <DropdownMenuGroup>
          <DropdownMenuLabel>Filter by tag</DropdownMenuLabel>
          {tags?.map((tag) => (
            <DropdownMenuCheckboxItem
              key={tag.name}
              checked={picked.includes(tag.name)}
              onCheckedChange={() => toggle(tag.name)}
            >
              {tag.name}
            </DropdownMenuCheckboxItem>
          ))}
          {tags?.length === 0 ? (
            <DropdownMenuItem disabled>No tags yet</DropdownMenuItem>
          ) : null}
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={picked.length === 0}
          onClick={() => onPick([])}
        >
          Clear filters
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function NoTaggedDashboards({ onClear }: { onClear: () => void }) {
  return (
    <div
      role="status"
      className="flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border bg-muted/30 px-4 py-6 text-center"
    >
      <span className={cn(TEXT_LABEL, "text-muted-foreground")}>
        No dashboard carries these tags.
      </span>
      <Button type="button" variant="outline" size="sm" onClick={onClear}>
        Clear filters
      </Button>
    </div>
  );
}
