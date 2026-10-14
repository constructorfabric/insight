import { useState } from "react";

import { DatePicker } from "@/components/ui/date-picker";
import { Input } from "@/components/ui/input";
import {
  isTime,
  momentParts,
  momentText,
  pickedDay,
  pickerDate,
} from "@/lib/alerts/moment";
import { TEXT_LABEL } from "@/lib/type-scale";

import { described } from "./condition-aria";

/**
 * A UTC day and time of day. A stored timestamp is shown to the minute and
 * written back only once the day or time is changed, so its seconds survive.
 */
export function MomentInput({
  id,
  label,
  value,
  describedBy,
  onChange,
}: {
  id: string;
  label: string;
  value: unknown;
  describedBy: readonly string[];
  onChange: (value: string) => void;
}) {
  const stored = momentParts(value);
  const [day, setDay] = useState(stored?.day ?? "");
  const [time, setTime] = useState(stored?.time ?? "");
  const shownDay = stored?.day ?? day;
  const shownTime = stored?.time ?? time;

  function write(nextDay: string, nextTime: string) {
    setDay(nextDay);
    setTime(nextTime);
    if (nextDay === "") return;

    onChange(momentText(nextDay, isTime(nextTime) ? nextTime : "00:00"));
  }

  return (
    // WORKAROUND: the kit's DatePicker forwards no aria-describedby, so the
    // refusal describes the group around the day and the time.
    <span
      role="group"
      aria-label={label}
      {...described(describedBy)}
      className="flex flex-wrap items-center gap-2"
    >
      <DatePicker
        id={id}
        aria-label={`${label} day`}
        selected={pickerDate(shownDay)}
        onSelect={(date) => {
          if (date) write(pickedDay(date), shownTime);
        }}
        className="h-9 w-40"
      />
      <Input
        id={`${id}-time`}
        aria-label={`${label} time`}
        type="time"
        value={shownTime}
        className="h-9 w-28 tabular-nums"
        {...described(describedBy)}
        onChange={(event) => write(shownDay, event.target.value)}
      />
      <span className={TEXT_LABEL}>UTC</span>
    </span>
  );
}
