import { FileCode2, Sparkles } from 'lucide-react';
import type { IssueDetail } from '../../../api/errorTracking/types';
import { IssueFix } from '../../assist/IssueAssist';

/** Where the fix belongs, and a fix proposed by Assist Code in the service's repository. */
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
        <IssueFix issue={issue} />
      </section>
    </div>
  );
}
