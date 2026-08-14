import { useEffect, useState } from "react";

/**
 * The value as of `delayMs` ago — the standard picker debounce, so a query
 * fires per pause in typing rather than per keystroke.
 */
export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}
