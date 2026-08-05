/**
 * Display labels for percentage shares of a whole (largest-remainder /
 * Hamilton rounding), summing to exactly 100 so a "parts of a whole" legend
 * never reads 99% or 101%. Shares are integers until a nonzero part would
 * display "0%" — then the whole legend switches to tenth-of-a-percent
 * precision (still largest-remainder, still summing to exactly 100) with a
 * 0.1 floor for nonzero parts, so one message beside 1,576 reads "0.1%",
 * never a contradictory "0%". Parts are assumed non-negative; a non-positive
 * total has no honest share and yields all "0". Order is preserved
 * (result[i] labels values[i]); labels carry no "%" suffix.
 */
export function percentShareLabels(values: number[]): string[] {
  const total = values.reduce((sum, value) => sum + value, 0);
  if (!(total > 0)) return values.map(() => "0");

  const integers = largestRemainder(values, total, 100);
  const needsTenths = values.some(
    (value, index) => value > 0 && integers[index] === 0,
  );
  if (!needsTenths) return integers.map(String);

  const tenths = largestRemainder(values, total, 1000);
  // Floor nonzero parts at one tenth, taking it from the largest share so
  // the sum stays exactly 100.0.
  for (let index = 0; index < values.length; index += 1) {
    if (!((values[index] ?? 0) > 0) || tenths[index] !== 0) continue;
    const largest = tenths.indexOf(Math.max(...tenths));
    if (largest === -1 || (tenths[largest] ?? 0) <= 1) break;
    tenths[largest] = (tenths[largest] ?? 0) - 1;
    tenths[index] = 1;
  }
  return tenths.map((share) =>
    share % 10 === 0 ? String(share / 10) : (share / 10).toFixed(1),
  );
}

/** Largest-remainder apportionment of `units` across `values`. */
function largestRemainder(
  values: number[],
  total: number,
  units: number,
): number[] {
  const exact = values.map((value) => (value / total) * units);
  const shares = exact.map((value) => Math.floor(value));
  let remainder = units - shares.reduce((sum, value) => sum + value, 0);

  // Hand the leftover units to the largest fractional parts first.
  const byFraction = exact
    .map((value, index) => ({ index, frac: value - Math.floor(value) }))
    .sort((a, b) => b.frac - a.frac);
  for (let i = 0; i < byFraction.length && remainder > 0; i += 1) {
    shares[byFraction[i]!.index] += 1;
    remainder -= 1;
  }
  return shares;
}
