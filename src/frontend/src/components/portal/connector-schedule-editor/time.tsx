import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@gears-frontx/ui-kit";

const HOURS = Array.from({ length: 24 }, (_, hour) =>
  String(hour).padStart(2, "0")
);
const MINUTES = Array.from({ length: 60 }, (_, minute) =>
  String(minute).padStart(2, "0")
);

export function TimeSelect({
  value,
  label,
  disabled,
  onChange,
}: {
  value: string;
  label: string;
  disabled: boolean;
  onChange: (time: string) => void;
}) {
  const [hour = "", minute = ""] = value.split(":");
  return (
    <div
      role="group"
      aria-label={label}
      className="flex max-w-full min-w-0 items-center gap-1"
    >
      <TimePart
        label="Hour"
        value={hour}
        options={HOURS}
        disabled={disabled}
        onChange={(next) => onChange(`${next}:${minute}`)}
      />
      <span aria-hidden="true">:</span>
      <TimePart
        label="Minute"
        value={minute}
        options={MINUTES}
        disabled={disabled}
        onChange={(next) => onChange(`${hour}:${next}`)}
      />
    </div>
  );
}

function TimePart({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  options: readonly string[];
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <Select
      value={value || null}
      disabled={disabled}
      onValueChange={(next) => {
        if (typeof next === "string") onChange(next);
      }}
    >
      <SelectTrigger
        size="sm"
        aria-label={label}
        className="h-8 w-[64px] max-w-full min-w-0 tabular-nums"
      >
        <SelectValue>{value || "—"}</SelectValue>
      </SelectTrigger>
      <SelectContent align="start" className="tabular-nums">
        {options.map((option) => (
          <SelectItem key={option} value={option}>
            {option}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
