/**
 * Synthetic ingestion-intensity responses for mock, Storybook and
 * `VITE_ENABLE_MOCKS=true` dev runs.
 *
 * Generated rather than captured, but CALIBRATED against a real read of the
 * gold ops view so the surface is exercised with the shape it will meet:
 *
 *   30d @ 15m, series=total  — 2884 buckets, per-bucket total 1 … ~6800
 *                              (nearly four orders of magnitude, which is the
 *                              whole reason that chart is on a log axis)
 *   24h @ 15m, by connector  — 97 buckets, three connectors ~61/32/7 %
 *   30m @ 1s,  by connector  — ~1800 buckets, per-bucket total 1 … ~13
 *
 * The pieces that matter for the lens are deliberate, not incidental: buckets
 * holding exactly ONE row (the case a log axis floored at 1 erases), and idle
 * buckets omitted entirely (the case a categorical x-axis would close up).
 *
 * Deterministic: every value derives from the bucket's own timestamp, so a
 * re-render and a reload draw the same chart and a screenshot diff means
 * something.
 */

import type {
  IngestionGrain,
  IngestionIntensity,
  IngestionPoint,
  IngestionSeries,
} from "@/api/ingestion-client";

/** Bands and their share of the total, as the calibration read them. */
const CONNECTORS: ReadonlyArray<readonly [string, number]> = [
  ["demo_tasks", 0.61],
  ["demo_chat", 0.32],
  ["demo_docs", 0.07],
];

const STREAMS_BY_CONNECTOR: Record<string, ReadonlyArray<readonly [string, number]>> = {
  bronze_demo_tasks: [
    ["issues", 0.59],
    ["comments", 0.28],
    ["boards", 0.13],
  ],
  bronze_demo_chat: [
    ["messages", 0.87],
    ["channels", 0.13],
  ],
  bronze_demo_docs: [["pages", 1]],
};

const MS = { "15m": 15 * 60 * 1_000, "1s": 1_000 } as const;

/** Peak rows in one 15-minute bucket, across all bands. */
const PEAK_15M = 6_800;

/** Peak rows in one 1-second bucket. */
const PEAK_1S = 13;

/** FNV-1a over the bucket, so a value depends only on its own timestamp. */
function noise(epoch: number, salt: number): number {
  let hash = 0x811c9dc5 ^ salt;
  const text = String(epoch);
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

/** 0…1, stable per bucket. */
function unit(epoch: number, salt: number): number {
  return (noise(epoch, salt) % 10_000) / 10_000;
}

/**
 * Rows in one bucket before the bands split it.
 *
 * Shaped like the real thing: a working-hours swell, a weekday/weekend
 * difference, idle stretches, and a thin tail of one-row buckets.
 */
function bucketTotal(epoch: number, grain: IngestionGrain): number {
  const at = new Date(epoch);
  const hour = at.getUTCHours();
  const weekday = at.getUTCDay() >= 1 && at.getUTCDay() <= 5;

  // Idle: no sync landed in this bucket at all. Omitted from the response, so
  // the chart shows a gap rather than a zero.
  if (unit(epoch, 1) < 0.04) return 0;
  // The sparse tail the log floor exists for.
  if (unit(epoch, 2) < 0.05) return 1;

  if (grain === "1s") {
    return 1 + Math.floor(unit(epoch, 3) * PEAK_1S);
  }
  const daily = hour >= 7 && hour <= 18 ? 1 : 0.12;
  const weekly = weekday ? 1 : 0.25;
  const jitter = 0.15 + unit(epoch, 4) * 0.85;
  return Math.max(1, Math.round(PEAK_15M * daily * weekly * jitter * 0.42));
}

function bandsFor(series: IngestionSeries, scope: string | null) {
  if (series === "total") return [["all", 1]] as ReadonlyArray<readonly [string, number]>;
  if (series === "stream") {
    return STREAMS_BY_CONNECTOR[scope ?? ""] ?? [["stream_a", 1]];
  }
  return CONNECTORS;
}

/**
 * Split a bucket total across bands.
 *
 * A band whose share rounds to nothing is dropped rather than sent as zero:
 * the real GROUP BY cannot emit a row for a band that extracted nothing, and a
 * zero would stack an invisible segment and appear in the legend.
 */
function splitBucket(
  epoch: number,
  total: number,
  bands: ReadonlyArray<readonly [string, number]>,
): Array<{ key: string; rows: number }> {
  const out: Array<{ key: string; rows: number }> = [];
  bands.forEach(([key, share], index) => {
    const wobble = 0.7 + unit(epoch, 10 + index) * 0.6;
    const rows = Math.round(total * share * wobble);
    if (rows > 0) out.push({ key, rows });
  });
  // Every bucket present in the response has at least one band.
  if (out.length === 0) out.push({ key: bands[0][0], rows: 1 });
  return out;
}

function stamp(epoch: number, grain: IngestionGrain): string {
  // The server emits `YYYY-MM-DD HH:MM:SS`, zone-less, and the 1s grain carries
  // milliseconds while the 15m grain does not.
  const iso = new Date(epoch).toISOString();
  return grain === "1s"
    ? `${iso.slice(0, 10)} ${iso.slice(11, 23)}`
    : `${iso.slice(0, 10)} ${iso.slice(11, 19)}`;
}

export interface IngestionMockRequest {
  grain: IngestionGrain;
  series?: IngestionSeries;
  scope?: string | null;
  from?: string | null;
  to?: string | null;
  now?: number;
}

/** The server's per-grain default window, so an unpinned request matches it. */
function defaultSpanMs(grain: IngestionGrain): number {
  return grain === "1s" ? 30 * 60 * 1_000 : 24 * 60 * 60 * 1_000;
}

export function buildIngestionIntensity(
  req: IngestionMockRequest,
): IngestionIntensity {
  const grain = req.grain;
  const scope = req.scope || null;
  const series: IngestionSeries = req.series ?? (scope ? "stream" : "connector");
  const now = req.now ?? Date.now();

  const to = req.to ? Date.parse(req.to) : now;
  const from = req.from ? Date.parse(req.from) : to - defaultSpanMs(grain);
  const step = MS[grain];

  const bands = bandsFor(series, scope);
  const points: IngestionPoint[] = [];
  // Aligned to the grain, the way toStartOfInterval() would.
  const first = Math.ceil(from / step) * step;
  for (let epoch = first; epoch < to; epoch += step) {
    const total = bucketTotal(epoch, grain);
    if (total === 0) continue;
    const bucket = stamp(epoch, grain);
    for (const band of splitBucket(epoch, total, bands)) {
      points.push({ bucket, key: band.key, rows: band.rows });
    }
  }

  const body: IngestionIntensity = {
    grain,
    series,
    from: new Date(from).toISOString().replace(/\.\d{3}Z$/, ".000Z"),
    to: new Date(to).toISOString().replace(/\.\d{3}Z$/, ".000Z"),
    truncated: false,
    points,
  };
  if (scope) body.scope = scope;
  return body;
}
