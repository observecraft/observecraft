// Client-side twin of the server's monitor message rendering (server/…/monitor_engine.rs), used to
// preview notifications and list recipients while editing.
import type { MonitorStatus } from '../../api/monitors/client';
import { fmtValue } from './common';

interface Ctx {
  status: MonitorStatus;
  previous: MonitorStatus;
  value: number | null;
  thresholds: { critical: number; warning?: number | null };
  monitorName: string;
  group: Record<string, string>;
}

export function renderMessage(template: string, ctx: Ctx): string {
  const recovery = ctx.status === 'ok' && ctx.previous !== 'ok';
  const flags: Record<string, boolean> = {
    is_alert: ctx.status === 'alert',
    is_warning: ctx.status === 'warn',
    is_no_data: ctx.status === 'nodata',
    is_recovery: recovery,
    is_alert_recovery: recovery && ctx.previous === 'alert',
    is_warning_recovery: recovery && ctx.previous === 'warn',
  };
  const shown: boolean[] = [];
  const visible = () => shown.every(Boolean);
  let out = '';
  let last = 0;
  for (const m of template.matchAll(/\{\{([^}]*)\}\}/g)) {
    if (visible()) out += template.slice(last, m.index);
    last = m.index! + m[0].length;
    const tag = m[1].trim();
    const block = /^([#^/])\s*(\w+)$/.exec(tag);
    if (block && block[2] in flags) {
      if (block[1] === '/') shown.pop();
      else shown.push(block[1] === '#' ? flags[block[2]] : !flags[block[2]]);
      continue;
    }
    if (!visible()) continue;
    const value =
      tag === 'value'
        ? ctx.value === null
          ? null
          : fmtValue(ctx.value)
        : tag === 'threshold'
          ? fmtValue(ctx.thresholds.critical)
          : tag === 'warn_threshold'
            ? ctx.thresholds.warning === null || ctx.thresholds.warning === undefined
              ? null
              : fmtValue(ctx.thresholds.warning)
            : tag === 'monitor.name'
              ? ctx.monitorName
              : tag.endsWith('.name')
                ? (ctx.group[tag.slice(0, -5)] ?? null)
                : null;
    out += value ?? m[0];
  }
  if (visible()) out += template.slice(last);
  return out;
}

/** `@handles` in a message (emails, @slack-…, @webhook-…, @pagerduty-…, @team-…, users). */
export function messageRecipients(message: string): string[] {
  const out: string[] = [];
  for (const m of message.matchAll(/(^|[^\w.+-])@([\w.+@-]+)/g)) {
    const h = m[2].replace(/[.@-]+$/, '');
    if (h && !out.includes(h)) out.push(h);
  }
  return out;
}
