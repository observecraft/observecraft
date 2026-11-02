// Links the curated catalog entries (OpenTelemetry / Vector templates, notification channels) to
// their integrations-core entry, so one tile carries both; integrations-core entries without a
// curated counterpart are listed on their own.
import type { CoreIntegration } from './catalog';

/** Curated ids that name their integration differently from integrations-core. */
const ALIASES: Record<string, string> = { postgresql: 'postgres' };

type CoreKey = Pick<CoreIntegration, 'id' | 'check' | 'name'>;

export function linkCore<C extends CoreKey>(curated: { id: string; name: string }[], core: C[]): { linked: Map<string, C>; unlinked: C[] } {
  const byKey = new Map<string, C>();
  for (const c of core) {
    byKey.set(`name:${c.name.toLowerCase()}`, c);
    byKey.set(`check:${c.check}`, c);
    byKey.set(`id:${c.id}`, c);
  }
  const linked = new Map<string, C>();
  for (const e of curated) {
    const id = ALIASES[e.id] ?? e.id;
    const match = byKey.get(`id:${id}`) ?? byKey.get(`check:${id.replace(/-/g, '_')}`) ?? byKey.get(`name:${e.name.toLowerCase()}`);
    if (match) linked.set(e.id, match);
  }
  const used = new Set(linked.values());
  return { linked, unlinked: core.filter((c) => !used.has(c)) };
}
