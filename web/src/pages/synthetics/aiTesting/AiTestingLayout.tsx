import { Link, NavLink, Outlet, useNavigate } from 'react-router-dom';
import { History, ListChecks, Plus, UserRound } from 'lucide-react';
import { useRuns } from '../../assist/runsQueries';
import { StatusBadge } from '../../assist/RunView';
import { timeAgo } from '../../../lib/format';
import { useProposals } from './queries';
import '../../assist/assist.css';
import './aiTesting.css';

/** Synthetics → AI Testing: new job, suites to review, all jobs and agent profiles beside the page. */
export function AiTestingLayout() {
  const navigate = useNavigate();
  const jobs = useRuns({ kind: 'testing', limit: 8 });
  const pending = useProposals({ status: 'pending' });
  const toReview = new Set((pending.data ?? []).map((p) => `${p.runId}\n${p.suite}`)).size;
  return (
    <div className="ait">
      <aside className="ait-side">
        <button className="btn btn-primary ait-new" onClick={() => navigate('/synthetics/ai-testing')}>
          <Plus size={15} /> New AI Testing Job
        </button>
        <nav className="ait-nav">
          <NavLink to="/synthetics/ai-testing/review" className={({ isActive }) => `ait-nav-link${isActive ? ' active' : ''}`}>
            <ListChecks size={15} /> Suites to review {toReview > 0 && <span className="ait-count">{toReview}</span>}
          </NavLink>
          <NavLink to="/synthetics/ai-testing/jobs" end className={({ isActive }) => `ait-nav-link${isActive ? ' active' : ''}`}>
            <History size={15} /> All jobs
          </NavLink>
          <NavLink to="/synthetics/ai-testing/profiles" className={({ isActive }) => `ait-nav-link${isActive ? ' active' : ''}`}>
            <UserRound size={15} /> Agent profiles
          </NavLink>
        </nav>
        {(jobs.data ?? []).length > 0 && (
          <div className="ait-recent">
            <h4>Recent jobs</h4>
            {(jobs.data ?? []).map((j) => (
              <NavLink key={j.id} to={`/synthetics/ai-testing/jobs/${j.id}`} className={({ isActive }) => `ait-recent-job${isActive ? ' active' : ''}`}>
                <span className="ait-recent-title">{j.title}</span>
                <span className="ait-recent-meta">
                  <StatusBadge status={j.status} /> <span className="muted">{timeAgo(j.createdAt)}</span>
                </span>
              </NavLink>
            ))}
          </div>
        )}
        <Link className="link-muted ait-docs" to="/synthetics/tests?q=type%3Abrowser">
          Browser tests
        </Link>
      </aside>
      <div className="ait-main">
        <Outlet />
      </div>
    </div>
  );
}
