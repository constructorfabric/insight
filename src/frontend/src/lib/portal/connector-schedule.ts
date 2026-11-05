import { Cron } from "croner";

const MINUTE_MS = 60_000;
const MAX_TIMEZONE_SHIFT_MS = 24 * 60 * MINUTE_MS;
const MAX_SCHEDULE_CANDIDATES = 8;

export function isScheduleOverride<T extends { crons: string[] }>(
  requested: T | null | undefined
): requested is T {
  return requested != null && requested.crons.length > 0;
}

function timezoneOffset(formatter: Intl.DateTimeFormat, date: Date): number {
  const parts = Object.fromEntries(
    formatter.formatToParts(date).map(({ type, value }) => [type, value])
  );
  const local = Date.UTC(
    Number(parts.year),
    Number(parts.month) - 1,
    Number(parts.day),
    Number(parts.hour),
    Number(parts.minute),
    Number(parts.second)
  );
  return local - Math.floor(date.getTime() / 1000) * 1000;
}

function repeatedRunAfter(
  cron: Cron,
  timezone: string,
  now: Date
): Date | null {
  if (timezone === "UTC") return null;

  const formatter = new Intl.DateTimeFormat("en-GB", {
    timeZone: timezone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  const before = new Date(now.getTime() - MAX_TIMEZONE_SHIFT_MS);
  const after = new Date(now.getTime() + MAX_TIMEZONE_SHIFT_MS);
  const shift =
    timezoneOffset(formatter, before) - timezoneOffset(formatter, after);
  if (shift <= 0 || shift > MAX_TIMEZONE_SHIFT_MS) return null;

  const start = Math.floor(now.getTime() / MINUTE_MS) * MINUTE_MS + MINUTE_MS;
  const end = now.getTime() + shift;
  for (let time = start; time <= end; time += MINUTE_MS) {
    const repeated = new Date(time);
    if (!cron.match(repeated)) continue;

    const previous = new Date(time - shift);
    const actualShift =
      timezoneOffset(formatter, previous) - timezoneOffset(formatter, repeated);
    if (actualShift === shift) return repeated;
  }
  return null;
}

function nextArgoRun(cron: Cron, timezone: string, now: Date): Date | null {
  if (cron.getPattern() == null) return null;

  const repeated = repeatedRunAfter(cron, timezone, now);
  let cursor = now;
  for (let attempt = 0; attempt < MAX_SCHEDULE_CANDIDATES; attempt++) {
    const next = cron.nextRun(cursor);
    if (next == null || next.getTime() <= cursor.getTime()) return repeated;

    // WORKAROUND: Argo skips missing local times and runs both occurrences of a repeated hour.
    if (cron.match(next)) {
      return repeated != null && repeated.getTime() < next.getTime()
        ? repeated
        : next;
    }
    cursor = next;
  }
  return repeated;
}

function parseArgoCron(expression: string, timezone: string): Cron | null {
  const withoutNames = expression.replace(
    /\b(?:JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC|SUN|MON|TUE|WED|THU|FRI|SAT)\b/gi,
    ""
  );
  if (!expression.trim().startsWith("@") && /[LW#]/i.test(withoutNames))
    return null;

  const fields = expression.trim().replaceAll("?", "*").split(/\s+/);
  for (const index of [2, 4]) {
    if (fields[index]?.split(",").some((item) => /^\*(?:\/0*1)?$/.test(item)))
      fields[index] = "*";
  }
  return new Cron(fields.join(" "), {
    ...(timezone === "UTC" ? { utcOffset: 0 } : { timezone }),
    mode: "5-part",
    domAndDow: false,
    sloppyRanges: true,
  });
}

export function nextScheduledRuns(
  expression: string,
  timezone: string,
  now: Date,
  count: number,
  until: number
): Date[] {
  const cron = parseArgoCron(expression, timezone);
  if (cron == null) return [];

  const result: Date[] = [];
  let cursor = now;
  for (let index = 0; index < count; index++) {
    const next = nextArgoRun(cron, timezone, cursor);
    if (next == null || next.getTime() > until) break;
    result.push(next);
    cursor = next;
  }
  return result;
}

export function nextScheduledRun(
  expression: string,
  timezone: string,
  now: Date
): Date | null {
  return nextScheduledRuns(expression, timezone, now, 1, Infinity)[0] ?? null;
}
