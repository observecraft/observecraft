import { useEffect, useState } from 'react';

/** `value` once it has stopped changing for `ms` (compared by JSON, so fresh objects with equal content don't restart it). */
export function useDebouncedValue<T>(value: T, ms: number): T {
  const key = JSON.stringify(value);
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(JSON.parse(key) as T), ms);
    return () => clearTimeout(t);
  }, [key, ms]);
  return settled;
}
