import { Tag, X } from "lucide-react";

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
          Clear tags
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function PickedTags({
  picked,
  onPick,
}: {
  picked: string[];
  onPick: (next: string[]) => void;
}) {
  if (picked.length === 0) return null;

  return (
    <ul
      aria-label="Picked tags"
      className="flex flex-wrap items-center gap-1.5"
    >
      {picked.map((name) => (
        <li
          key={name}
          className="inline-flex h-6 max-w-48 items-center gap-1 rounded-sm border ps-1.5 text-xs text-muted-foreground"
        >
          <Tag className="size-3 shrink-0" aria-hidden />
          <span className="truncate" title={name}>
            {name}
          </span>
          <button
            type="button"
            aria-label={`Remove tag ${name}`}
            className="inline-flex size-6 shrink-0 items-center justify-center rounded-sm hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            onClick={() => onPick(picked.filter((one) => one !== name))}
          >
            <X className="size-3" aria-hidden />
          </button>
        </li>
      ))}
    </ul>
  );
}
