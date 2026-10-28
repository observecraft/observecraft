import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import { catalogHealth } from '../../../api/portalCatalog/reports';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { useTime } from '../../../components/shell/TimeContext';
import { KIND_LABEL } from '../catalog/catalogModel';
import { useOnCallTeams, usePortalCatalog } from '../catalog/queries';
import { EntityIcon } from '../catalog/visuals';
import { useReports } from './ReportsLayout';

const pct = (covered: number, total: number) => (total ? Math.round((covered * 100) / total) : 0);

/** Reports → Catalog Health: how many services and defined entities have an owner, docs, a repository, contacts and On-Call. */
export function CatalogHealth() {
  const { teams } = useReports();
  const { range } = useTime();
  const catalog = usePortalCatalog(range, null);
  const onCall = useOnCallTeams();
  const health = useMemo(() => {
    const withOnCall = new Set((onCall.data ?? []).filter((t) => t.schedules.length > 0).map((t) => t.handle));
    return catalogHealth(catalog.data?.entities ?? [], withOnCall, teams);
  }, [catalog.data, onCall.data, teams]);
  if (catalog.isPending) return <Loading label="Loading the catalog…" />;
  if (catalog.isError) return <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />;
  const gaps = health.entities
    .map((e) => ({
      e,
      missing: [!e.owner && 'owner', !e.docsUrl && !e.links.some((l) => l.type === 'doc' || l.type === 'runbook') && 'docs', !e.repoUrl && 'repository', !e.contacts.length && 'contacts'].filter(
        (m): m is string => !!m,
      ),
    }))
    .filter((g) => g.missing.length)
    .sort((a, b) => b.missing.length - a.missing.length || a.e.name.localeCompare(b.e.name));
  const overall = health.rows.length ? Math.round(health.rows.reduce((n, r) => n + pct(r.covered, r.total), 0) / health.rows.length) : 0;

  return (
    <div className="pr-report">
      <h1 className="pr-title">Catalog Health</h1>
      <p className="pr-muted pr-lead">Services and the entities your teams defined ({health.entities.length}). Endpoints, repositories and dependencies seen only in telemetry are left out.</p>
      <div className="pr-cards">
        <section className="pr-card">
          <div className="pr-card-title">Overall completeness</div>
          <div className="pr-big">{overall}%</div>
          <p className="pr-muted">Mean of the checks on the right.</p>
        </section>
        <section className="pr-card pr-card-wide">
          <div className="pr-card-title">Entities with…</div>
          <ul className="pr-bars">
            {health.rows.map((r) => (
              <li key={r.key}>
                <span className="pr-bar-label">{r.label}</span>
                <span className="pr-bar">
                  <span style={{ width: `${pct(r.covered, r.total)}%` }} />
                </span>
                <span className="pr-bar-value">
                  {pct(r.covered, r.total)}%{' '}
                  <span className="pr-muted">
                    ({r.covered}/{r.total})
                  </span>
                </span>
              </li>
            ))}
          </ul>
        </section>
      </div>
      <section className="pr-section">
        <header className="pr-section-head">
          <h2>What's missing</h2>
          <span className="pr-muted">{gaps.length} entities</span>
        </header>
        {gaps.length ? (
          <div className="pr-table-wrap">
            <table className="pr-table">
              <thead>
                <tr>
                  <th>Entity</th>
                  <th>Type</th>
                  <th>Team</th>
                  <th>Missing</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {gaps.slice(0, 100).map(({ e, missing }) => (
                  <tr key={e.id}>
                    <td>
                      <span className="pr-name">
                        <EntityIcon entity={e} /> <Link to={`/portal/catalog?entity=${encodeURIComponent(e.id)}`}>{e.displayName ?? e.name}</Link>
                      </span>
                    </td>
                    <td>{KIND_LABEL[e.kind]}</td>
                    <td>{e.owner || <span className="pr-muted">No team</span>}</td>
                    <td>{missing.join(', ')}</td>
                    <td>
                      <Link className="btn btn-sm" to={e.definitionId ? `/portal/catalog/edit/${e.definitionId}` : `/portal/catalog/new?kind=${e.kind}&name=${encodeURIComponent(e.name)}`}>
                        {e.definitionId ? 'Edit' : 'Add definition'}
                      </Link>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="pr-muted">Every entity has an owner, docs, a repository and contacts.</p>
        )}
      </section>
    </div>
  );
}
