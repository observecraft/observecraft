import { useCallback, useMemo, useRef, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { ArrowDownToLine, ArrowRight, Copy, Eye, MonitorSmartphone, Plus, SendHorizontal, Sparkles } from 'lucide-react';
import type { IssueDetail, QuerySuggestion } from '../../../api/errorTracking/types';
import type { TimeRange } from '../../../api/types';
import { Chart } from '../../../components/charts/Chart';
import { QueryBar } from '../../../components/query/QueryBar';
import { TimePicker } from '../../../components/shell/PageHeader';
import { Select } from '../../../components/ui/Dropdown';
import { formatLongDate, timeAgo } from '../../../lib/format';
import { addTerm } from '../../../lib/searchQuery';
import { token } from '../../../theme/chartTheme';
import { AssigneeSelect } from '../issueControls';
import { Avatar } from '../issueVisuals';
import { useAddComment, useTeams } from '../queries';
import { ErrorSampleView } from './ErrorSampleView';

interface Props {
  issue: IssueDetail;
  range: TimeRange;
  isRefreshing: boolean;
}

function hourLabel(t: number): string {
  const d = new Date(t);
  if (d.getHours() === 0 && d.getMinutes() < 30) return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

function histogramOption(issue: IssueDetail): EChartsOption {
  return {
    grid: { left: 24, right: 8, top: 10, bottom: 22 },
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
    xAxis: {
      type: 'category',
      data: issue.histogram.map((b) => b.t),
      axisLabel: { formatter: (v: string) => hourLabel(Number(v)), interval: 5 },
    },
    yAxis: { type: 'value', minInterval: 1, splitNumber: 2, splitLine: { show: false } },
    series: [
      {
        type: 'bar',
        color: token('--status-error'),
        barMaxWidth: 10,
        data: issue.histogram.map((b) => b.count),
        markLine: {
          symbol: 'none',
          silent: true,
          label: { show: false },
          lineStyle: { type: 'dashed', color: token('--text-muted') },
          data: [{ xAxis: issue.histogram.length - 1 }],
        },
      },
    ],
  };
}

export function ReviewTab({ issue, range, isRefreshing }: Props) {
  const [query, setQuery] = useState('');
  const [investigate, setInvestigate] = useState(false);
  const sampleRef = useRef<HTMLDivElement>(null);
  const option = useMemo(() => histogramOption(issue), [issue]);
  const goToSample = () => sampleRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' });

  // Panel query suggestions come from the issue's own tag distribution.
  const suggest = useCallback(
    async (partial: string): Promise<QuerySuggestion[]> =>
      issue.tags.map((t) => ({ text: `${t.key}:${t.value}`, count: Math.round(t.share * issue.count) })).filter((s) => s.text.toLowerCase().includes(partial.toLowerCase())),
    [issue],
  );

  return (
    <div className={`ip-review${isRefreshing ? ' is-stale' : ''}`}>
      <div className="investigate-banner">
        <Sparkles size={16} className="brand-icon" />
        <strong>Assist Investigation</strong>
        <span className="muted-text">Investigate this issue, find the root cause, and generate a fix.</span>
        <button className="btn btn-brand" onClick={() => setInvestigate(true)}>
          <Sparkles size={13} /> Investigate this issue
        </button>
      </div>
      {investigate && (
        <div className="investigate-note">
          AI investigation isn't connected yet. Once the backend is in place, this will analyse the stack trace, recent deploys and correlated logs for {issue.service}.
        </div>
      )}

      <div className="ip-filter-row">
        <div className="ip-filter-query">
          <QueryBar value={query} onChange={setQuery} suggest={suggest} placeholder="Filter errors" />
        </div>
        <button className="icon-square" title="Copy query" onClick={() => navigator.clipboard?.writeText(query)}>
          <Copy size={14} />
        </button>
        <TimePicker compact />
      </div>

      <div className="ip-grid">
        <section className="ip-card ip-summary">
          <div className="ip-seen">
            <span>
              <strong>First seen</strong> <u title={formatLongDate(issue.firstSeen)}>{timeAgo(issue.firstSeen)}</u>
              {issue.firstSeenVersion && <span className="version-chip">{issue.firstSeenVersion}</span>}
            </span>
            <ArrowRight size={16} className="ip-seen-arrow" />
            <span>
              <strong>Last seen</strong> <u title={formatLongDate(issue.lastSeen)}>{timeAgo(issue.lastSeen)}</u>
              {issue.lastSeenVersion && <span className="version-chip">{issue.lastSeenVersion}</span>}
            </span>
            <button className="btn btn-outline-primary" onClick={goToSample}>
              <ArrowDownToLine size={14} /> Go to Error Sample
            </button>
          </div>
          <div className="ip-volume">
            <div className="ip-count">
              <span>{issue.count.toLocaleString()}</span>
              <button className="link-btn" onClick={goToSample}>
                View All Errors
              </button>
            </div>
            <div className="ip-histogram">
              <Chart option={option} height={110} />
            </div>
          </div>
          {issue.impact && (
            <div className="ip-row">
              <span className="ip-row-label">Impact</span>
              <span className="impact-chip">
                <MonitorSmartphone size={14} /> {issue.impact.sessions.toLocaleString()} sessions
                <span className="muted-text">{issue.impact.sessionShare < 0.01 ? '< 1%' : `${(issue.impact.sessionShare * 100).toFixed(1)}%`}</span>
              </span>
              <span className="impact-chip">
                <Eye size={14} /> {issue.impact.views} views
              </span>
            </div>
          )}
          <div className="ip-row">
            <span className="ip-row-label">Tags</span>
            <div className="ip-tags">
              {issue.tags.map((t) => (
                <button key={t.key} className="dist-chip" onClick={() => setQuery((q) => addTerm(q, t.key, t.value))} title={`Filter samples by ${t.key}:${t.value}`}>
                  {t.key}:{t.value} <strong>({Math.round(t.share * 100)}%)</strong>
                </button>
              ))}
            </div>
          </div>
        </section>

        <aside className="ip-side">
          <Ownership issue={issue} />
          <Comments issue={issue} />
        </aside>
      </div>

      <div ref={sampleRef}>
        <ErrorSampleView issue={issue} range={range} query={query} />
      </div>
    </div>
  );
}

function Ownership({ issue }: { issue: IssueDetail }) {
  const teams = useTeams();
  const [team, setTeam] = useState<string>(issue.team ?? '');
  return (
    <section className="ip-card ip-owner">
      <header>
        <strong>Ownership</strong>
        <Select
          value={team}
          options={[{ value: '', label: 'No team' }, ...(teams.data ?? []).map((t) => ({ value: t, label: t }))]}
          onChange={setTeam}
          align="right"
          triggerClassName="link-btn"
          trigger={() => (
            <>
              <Plus size={14} /> Add team
            </>
          )}
        />
      </header>
      <dl>
        <dt>Team</dt>
        <dd className={team ? '' : 'muted-text'}>{team || 'No team identified'}</dd>
        <dt>Assigned to</dt>
        <dd>
          <AssigneeSelect issueId={issue.id} assignee={issue.assignee} label="Assign" />
        </dd>
      </dl>
    </section>
  );
}

function Comments({ issue }: { issue: IssueDetail }) {
  const add = useAddComment(issue.id);
  const [text, setText] = useState('');
  const comments = issue.activity.filter((a) => a.kind === 'comment');
  const submit = () => {
    if (!text.trim()) return;
    add.mutate(text.trim(), { onSuccess: () => setText('') });
  };
  return (
    <section className="ip-card ip-comments">
      {comments.map(
        (c) =>
          c.kind === 'comment' && (
            <div className="comment" key={c.at}>
              <Avatar user={c.by} size={22} />
              <div>
                <div className="comment-meta">
                  <strong>{c.by.name}</strong> <span className="muted-text">{timeAgo(c.at)}</span>
                </div>
                <div>{c.text}</div>
              </div>
            </div>
          ),
      )}
      <div className="comment-box">
        <textarea
          placeholder={comments.length ? 'Write a comment' : 'Write the first comment'}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && (e.metaKey || e.ctrlKey) && submit()}
        />
        <button className="comment-send" title="Send (⌘↵)" disabled={!text.trim() || add.isPending} onClick={submit}>
          <SendHorizontal size={14} />
        </button>
      </div>
    </section>
  );
}
