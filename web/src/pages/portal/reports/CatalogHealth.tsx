import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { Copy } from 'lucide-react';
import type { PortalEntity } from '../../../api/portalCatalog/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Select } from '../../../components/ui/Dropdown';
import { useTime } from '../../../components/shell/TimeContext';
import { usePortalCatalog } from '../catalog/queries';
import { useReports } from './ReportsLayout';
import { BigNumber, CountTreemap, DonutWithLegend, ReportSection } from './widgets';

/** The entity kinds the report counts, as they read in it. */
const KINDS: [PortalEntity['kind'], string][] = [
  ['system', 'Systems'],
  ['service', 'Services'],
  ['api', 'APIs'],
  ['queue', 'Queues'],
  ['frontend', 'Frontend Apps'],
];

/** Entities defined in the catalog are in its definition format, v3; the others have none. */
const definitionVersion = (e: PortalEntity) => (e.definitionId ? 'v3' : '');

/** Values of a field with their counts, the most common first ('' is "none"). */
function countBy(entities: PortalEntity[], field: (e: PortalEntity) => string | null | undefined) {
  const counts = new Map<string, number>();
  for (const e of entities) {
    const v = field(e) ?? '';
    if (v) counts.set(v, (counts.get(v) ?? 0) + 1);
  }
  return [...counts].map(([label, value]) => ({ label, value })).sort((a, b) => b.value - a.value);
}

const ALL = '*';

/** Reports → Catalog Health: what the catalog holds, how complete its definitions are and who owns it. */
export function CatalogHealth() {
  const { range } = useTime();
  const { teams } = useReports();
  const catalog = usePortalCatalog(range, null);
  const [owner, setOwner] = useState(teams[0] ?? ALL);
  const [lifecycle, setLifecycle] = useState(ALL);
  const [tier, setTier] = useState(ALL);
  const [version, setVersion] = useState(ALL);
  const all = useMemo(() => (catalog.data?.entities ?? []).filter((e) => KINDS.some(([k]) => k === e.kind)), [catalog.data]);
  const entities = all.filter(
    (e) =>
      (owner === ALL || e.owner === owner) &&
      (lifecycle === ALL || (e.lifecycle ?? '') === lifecycle) &&
      (tier === ALL || (e.tier ?? '') === tier) &&
      (version === ALL || definitionVersion(e) === version),
  );
  if (catalog.isPending) return <Loading label="Loading the catalog…" />;
  if (catalog.isError) return <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />;

  const options = (values: string[]) => [{ value: ALL, label: '*' }, ...values.filter(Boolean).map((v) => ({ value: v, label: v }))];
  const ofKind = (list: PortalEntity[], kind: string) => list.filter((e) => e.kind === kind).length;
  const unowned = entities.filter((e) => !e.owner);

  return (
    <div className="pr-report">
      <div className="rw-filters">
        <Select label="owner" value={owner} options={options([...new Set(all.map((e) => e.owner))].sort())} onChange={setOwner} />
        <Select label="lifecycle" value={lifecycle} options={options([...new Set(all.map((e) => e.lifecycle ?? ''))].sort())} onChange={setLifecycle} />
        <Select label="tier" value={tier} options={options([...new Set(all.map((e) => e.tier ?? ''))].sort())} onChange={setTier} />
        <Select label="definition_api_version" value={version} options={options(['v3'])} onChange={setVersion} />
      </div>
      <div className="rw-title-row">
        <h1 className="pr-title">Catalog Health</h1>
        <Link className="link-btn" to="/dashboards">
          <Copy size={14} /> Clone as Dashboard
        </Link>
      </div>

      <ReportSection id="summary" title="Summary Information">
        <div className="rw-tiles rw-tiles-6">
          <BigNumber title="Total Entities" value={entities.length} />
          {KINDS.map(([kind, label]) => (
            <BigNumber key={kind} title={label} value={ofKind(entities, kind)} />
          ))}
        </div>
        <DonutWithLegend title="Entity Types in Catalog" slices={KINDS.map(([kind, label]) => ({ label, value: ofKind(entities, kind) }))} />
      </ReportSection>

      <ReportSection id="stats" title="Stats on Catalog Entities">
        <div className="rw-tiles rw-tiles-3 rw-tall">
          <CountTreemap title="Entities by Lifecycle" slices={countBy(entities, (e) => e.lifecycle)} />
          <CountTreemap title="Entities by Tier" slices={countBy(entities, (e) => e.tier)} />
          <CountTreemap title="Entities by Definition Version" slices={countBy(entities, definitionVersion)} />
        </div>
        <div className="rw-tiles rw-tiles-3">
          <BigNumber title="Entities with no Lifecycle" value={entities.filter((e) => !e.lifecycle).length} />
          <BigNumber title="Entities with no Tier" value={entities.filter((e) => !e.tier).length} />
          <BigNumber title="Entities with no Definition Version" value={entities.filter((e) => !definitionVersion(e)).length} />
        </div>
      </ReportSection>

      <ReportSection id="without-owners" title="Entities Without Owners">
        <div className="rw-tiles rw-tiles-6">
          <BigNumber title="Total" value={unowned.length} />
          {KINDS.map(([kind, label]) => (
            <BigNumber key={kind} title={label} value={ofKind(unowned, kind)} />
          ))}
        </div>
      </ReportSection>

      <ReportSection id="ownership" title="Entity Ownership Information">
        <DonutWithLegend
          title="Ownership Distribution"
          slices={[
            { label: 'Without Owners', value: unowned.length },
            { label: 'With Owners', value: entities.length - unowned.length },
          ]}
        />
      </ReportSection>
    </div>
  );
}
