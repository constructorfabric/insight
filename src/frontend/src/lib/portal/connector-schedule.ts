import { Cron } from "croner";

const MINUTE_MS = 60_000;
const MAX_TIMEZONE_SHIFT_MS = 24 * 60 * MINUTE_MS;
const MAX_SCHEDULE_CANDIDATES = 8;

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

export function nextScheduledRun(
  expression: string,
  timezone: string,
  now: Date
): Date | null {
  const withoutNames = expression.replace(
    /\b(?:JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC|SUN|MON|TUE|WED|THU|FRI|SAT)\b/gi,
    ""
  );
  if (!expression.trim().startsWith("@") && /[LW#]/i.test(withoutNames))
    return null;

  const cron = new Cron(expression.replaceAll("?", "*"), {
    timezone,
    mode: "5-part",
    domAndDow: false,
    sloppyRanges: true,
  });
  return nextArgoRun(cron, timezone, now);
}
