import type { TimeRange } from '../api/types';

const MINUTE = 60_000;

/** Live ranges move every 10s; snapping to the minute keeps query keys (and loaded pages) stable. */
export function snapRange(r: TimeRange): TimeRange {
  return { from: Math.floor(r.from / MINUTE) * MINUTE, to: Math.floor(r.to / MINUTE) * MINUTE };
}

/** The end of a `windowMs` range centered on `center`, or now when that would be in the future. */
export function centeredEnd(center: number, windowMs: number, now: number): number {
  return Math.min(now, center + windowMs / 2);
}
