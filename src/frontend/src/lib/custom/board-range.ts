import { isRangeToken, toInterval } from "./time-range";

/**
 * Which window a board opens on: what the URL asked for, else the board's own
 * default, else the first window it offers.
 *
 * A board offering none has no picker and no window — it reads every row, as
 * every board did before ranges existed.
 */
export function selectedRange(
  offered: string[] | undefined,
  fallback: string | undefined,
  fromUrl: string | undefined,
): string | undefined {
  if (!offered || offered.length === 0) return undefined;

  const usable = (token: string | undefined): token is string =>
    typeof token === "string" &&
    isRangeToken(token) &&
    (offered.includes(token) || toInterval(token) !== undefined);

  if (usable(fromUrl)) return fromUrl;
  if (usable(fallback)) return fallback;

  return offered[0];
}
