import { useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowRight, Boxes, Search, Server, Star } from 'lucide-react';
import { age, entitiesOf, type EntityTab } from '../../api/portal/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { HomeSection } from './homeCommon';
import { useEntities, useSetFavorite } from './homeQueries';

const TABS: { key: EntityTab; label: string }[] = [
  { key: 'recents', label: 'Recents' },
  { key: 'teams', label: 'Your Teams' },
  { key: 'favorites', label: 'Favorites' },
];

const count = (n: number) => (n > 0 ? n : <span className="sc-muted">0</span>);

/** Home → Services & Entities: the member's recent, team and starred services with what exists around them. */
export function HomeEntities({ myTeams }: { myTeams: string[] }) {
  const [tab, setTab] = useState<EntityTab>('recents');
  const q = useEntities();
  const favorite = useSetFavorite();
  const rows = q.data ? entitiesOf(q.data, tab, myTeams) : [];
  return (
    <HomeSection
      id="entities"
      icon={<Boxes size={16} />}
      title="Services & Entities"
      actions={
        <Link to="/portal/catalog" className="ph-link">
          Go to Software Catalog <ArrowRight size={13} />
        </Link>
      }
    >
      <div className="ph-tabs" role="tablist">
        {TABS.map((t) => (
          <button key={t.key} role="tab" aria-selected={t.key === tab} className={`ph-tab${t.key === tab ? ' active' : ''}`} onClick={() => setTab(t.key)}>
            {t.label}
          </button>
        ))}
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.error ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : rows.length === 0 ? (
        <div className="ph-no-results">
          <Search size={44} strokeWidth={1.4} />
          <strong>No matching results found</strong>
          <span>{tab === 'favorites' ? 'Star a service to keep it here.' : tab === 'teams' ? 'No service is owned by your teams yet.' : 'No service sent spans recently.'}</span>
        </div>
      ) : (
        <table className="ph-grid-table ph-entities">
          <thead>
            <tr>
              <th style={{ width: 34 }} aria-label="Favorite" />
              <th style={{ width: 90 }}>Type</th>
              <th>Name</th>
              <th style={{ width: 110 }}>Last deploy</th>
              <th style={{ width: 86 }}>Monitors</th>
              <th style={{ width: 70 }}>Issues</th>
              <th style={{ width: 96 }}>Dashboards</th>
              <th style={{ width: 96 }}>Scorecards</th>
              <th style={{ width: 150 }}>On-call</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((e) => (
              <tr key={e.name}>
                <td>
                  <button
                    className={`ph-star${e.favorite ? ' on' : ''}`}
                    aria-pressed={e.favorite}
                    aria-label={e.favorite ? `Remove ${e.name} from favorites` : `Add ${e.name} to favorites`}
                    onClick={() => favorite.mutate({ entity: e.name, favorite: !e.favorite })}
                  >
                    <Star size={14} />
                  </button>
                </td>
                <td>
                  <span className="ph-type">
                    <Server size={13} /> Service
                  </span>
                </td>
                <td>
                  <Link to={`/apm/services/${encodeURIComponent(e.name)}`} className="ph-entity-name">
                    {e.name}
                  </Link>
                  {e.team && <span className="ph-team-chip">@{e.team}</span>}
                </td>
                <td className="sc-muted">{age(e.lastDeploy)}</td>
                <td>{count(e.monitors)}</td>
                <td>{e.issues > 0 ? <span className="ph-issues">{e.issues}</span> : count(0)}</td>
                <td>{count(e.dashboards)}</td>
                <td>{e.scorecardLevel == null ? <span className="sc-muted">—</span> : <span className={`sc-level sc-level-${e.scorecardLevel}`}>L{e.scorecardLevel}</span>}</td>
                <td className="ph-ellipsis">{e.onCall ?? <span className="sc-muted">—</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </HomeSection>
  );
}
