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
  const [seen, setSeen] = useState(value);
  const [day, setDay] = useState(stored?.day ?? "");
  const [time, setTime] = useState(stored?.time ?? "");
  if (value !== seen) {
    setSeen(value);
    setDay(stored?.day ?? "");
    setTime(stored?.time ?? "");
  }

  // INVARIANT: an unfinished time stays in the field and writes nothing, so
  // the rule never takes a time nobody typed.
  function pickDay(nextDay: string) {
    const nextTime = isTime(time) ? time : "00:00";
    setDay(nextDay);
    setTime(nextTime);
    onChange(momentText(nextDay, nextTime));
  }

  function typeTime(nextTime: string) {
    setTime(nextTime);
    if (day === "" || !isTime(nextTime)) return;

    onChange(momentText(day, nextTime));
  }

  function leaveTime() {
    if (isTime(time)) return;

    setTime(stored?.time ?? (day === "" ? "" : "00:00"));
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
        selected={pickerDate(day)}
        onSelect={(date) => {
          if (date) pickDay(pickedDay(date));
        }}
        className="h-9 w-40"
      />
      <Input
        id={`${id}-time`}
        aria-label={`${label} time`}
        type="time"
        value={time}
        className="h-9 w-28 tabular-nums"
        {...described(describedBy)}
        onChange={(event) => typeTime(event.target.value)}
        onBlur={leaveTime}
      />
      <span className={TEXT_LABEL}>UTC</span>
    </span>
  );
}
