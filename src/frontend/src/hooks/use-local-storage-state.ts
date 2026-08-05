import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";

export interface LocalStorageStateOptions<T> {
  key?: string;
  defaultValue: T;
  parse: (raw: string) => T | undefined;
  serialize: (value: T) => string;
}

export function parseLocalStorageBoolean(value: string): boolean | undefined {
  if (value === "true") return true;
  if (value === "false") return false;
  return undefined;
}

export function serializeLocalStorageBoolean(value: boolean): string {
  return String(value);
}

function readValue<T>({
  key,
  defaultValue,
  parse,
}: Pick<LocalStorageStateOptions<T>, "key" | "defaultValue" | "parse">): T {
  if (!key || typeof window === "undefined") return defaultValue;
  try {
    const raw = window.localStorage.getItem(key);
    return raw == null ? defaultValue : (parse(raw) ?? defaultValue);
  } catch {
    return defaultValue;
  }
}

export function useLocalStorageState<T>(
  options: LocalStorageStateOptions<T>
): [T, Dispatch<SetStateAction<T>>] {
  const { key, defaultValue, parse, serialize } = options;
  const [value, setValue] = useState<T>(() =>
    readValue({ key, defaultValue, parse })
  );
  const activeKey = useRef(key);
  const shouldPersist = useRef(false);
  const updateValue = useCallback<Dispatch<SetStateAction<T>>>((next) => {
    shouldPersist.current = true;
    setValue(next);
  }, []);

  useEffect(() => {
    if (activeKey.current !== key) {
      activeKey.current = key;
      shouldPersist.current = false;
      setValue(readValue({ key, defaultValue, parse }));
      return;
    }
    if (!shouldPersist.current) return;
    shouldPersist.current = false;
    if (!key || typeof window === "undefined") return;
    try {
      window.localStorage.setItem(key, serialize(value));
    } catch {
      return;
    }
  }, [defaultValue, key, parse, serialize, value]);

  return [value, updateValue];
}
