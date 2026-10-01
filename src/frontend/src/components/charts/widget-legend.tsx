export interface LegendItem {
  label: string;
  color: string;
  style?: "line" | "dashed";
}

export function WidgetLegend({ items }: { items: LegendItem[] }) {
  if (items.length === 0) return null;

  return (
    <div
      role="list"
      aria-label="Chart legend"
      className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1.5 px-2 pt-1 pb-0.5 text-xs leading-snug text-muted-foreground"
    >
      {items.map((item) => (
        <span
          key={item.label}
          role="listitem"
          className="inline-flex items-center gap-1.5 whitespace-nowrap"
        >
          <Marker {...item} />
          {item.label}
        </span>
      ))}
    </div>
  );
}

function Marker({ color, style }: LegendItem) {
  if (!style) {
    return (
      <span
        aria-hidden="true"
        data-marker="dot"
        className="size-2 shrink-0 rounded-full"
        style={{ backgroundColor: color }}
      />
    );
  }

  return (
    <span
      aria-hidden="true"
      data-marker={style}
      className="h-0 w-3.5 shrink-0"
      style={{
        borderTop: `2px ${style === "dashed" ? "dashed" : "solid"} ${color}`,
      }}
    />
  );
}
