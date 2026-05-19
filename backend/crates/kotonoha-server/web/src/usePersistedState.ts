import { useEffect, useState } from "react";

/** `useState` + localStorage. Survives page reloads. */
export function usePersistedState<T>(
  key: string,
  fallback: T,
): [T, (v: T) => void] {
  const [val, setVal] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      if (raw !== null) return JSON.parse(raw) as T;
    } catch {
      // Ignore parse errors / private-mode storage failures.
    }
    return fallback;
  });

  useEffect(() => {
    try {
      localStorage.setItem(key, JSON.stringify(val));
    } catch {
      // Storage quota / private mode — best effort.
    }
  }, [key, val]);

  return [val, setVal];
}
