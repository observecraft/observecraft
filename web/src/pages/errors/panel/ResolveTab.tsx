import { FileCode2, GitPullRequest, Sparkles } from 'lucide-react';
import type { IssueDetail } from '../../../api/errorTracking/types';

/** Preview: where the fix belongs. Code suggestions need a source-code integration (not built yet). */
export function ResolveTab({ issue }: { issue: IssueDetail }) {
  return (
    <div className="resolve">
      <section className="ip-card">
        <h4>
          <FileCode2 size={15} /> Suspected location
        </h4>
        {issue.location ? (
          <p className="mono">
            {issue.location.function}
            {issue.location.file && <span className="muted-text"> in {issue.location.file}</span>}
          </p>
        ) : (
          <p className="muted-text">No in-app frame was identified for this issue.</p>
        )}
      </section>
      <section className="ip-card">
        <h4>
          <Sparkles size={15} className="brand-icon" /> Suggested fix
        </h4>
        <p className="muted-text">
          Connect a source code repository to get code-level fix suggestions and open a pull request from here.
        </p>
        <button className="btn" disabled>
          <GitPullRequest size={14} /> Connect repository
        </button>
      </section>
    </div>
  );
}
