const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

const pad = (n: number, w = 2) => String(n).padStart(w, '0');

/** 21900 → "21.9K", 5000 → "5k" style compact counts used in suggestions and axes. */
export function compact(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1e6) return `${+(n / 1e3).toFixed(1)}K`;
  return `${+(n / 1e6).toFixed(1)}M`;
}

/** "Oct 25 09:48:22.309" — the log-list timestamp format. */
export function formatLogDate(ts: number): string {
  const d = new Date(ts);
  return `${MONTHS[d.getMonth()]} ${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

/** "Oct 25, 2026 at 10:05:38.379 am" — the log side panel header. */
export function formatLongDate(ts: number): string {
  const d = new Date(ts);
  const h = d.getHours() % 12 || 12;
  return `${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()} at ${pad(h)}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)} ${d.getHours() < 12 ? 'am' : 'pm'}`;
}

/** "a few seconds ago", "3 minutes ago", "2 hours ago", "4 days ago", "4 months ago", "about 1 year ago". */
export function timeAgo(ts: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 45) return 'a few seconds ago';
  if (s < 90) return 'a minute ago';
  const m = Math.round(s / 60);
  if (m < 45) return `${m} minutes ago`;
  if (m < 90) return 'an hour ago';
  const h = Math.round(m / 60);
  if (h < 22) return `${h} hours ago`;
  const d = Math.round(h / 24);
  if (d <= 1) return 'a day ago';
  if (d < 45) return `${d} days ago`;
  if (d < 320) return `${Math.round(d / 30)} months ago`;
  if (d < 548) return 'about 1 year ago';
  return `${Math.round(d / 365)} years ago`;
}

export function hhmm(t: number): string {
  const d = new Date(t);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
