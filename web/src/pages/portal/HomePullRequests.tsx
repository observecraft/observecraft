import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { GitPullRequest, MessageSquare } from 'lucide-react';
import type { CodeHost, PortalPullRequest } from '../../api/portal/client';
import { age, groupPullRequests } from '../../api/portal/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { ConnectOverlay, HomeGroup, HomeSection, ProviderTabs } from './homeCommon';
import { usePullRequests } from './homeQueries';

type Tab = CodeHost | 'azure-devops';
const TABS: { key: Tab; label: string; logo: string }[] = [
  { key: 'github', label: 'GitHub', logo: 'github' },
  { key: 'gitlab', label: 'GitLab', logo: 'gitlab' },
  { key: 'azure-devops', label: 'Azure DevOps', logo: 'tile:azure-devops' },
];

/** Fictional rows drawn blurred behind the "connect" card. */
const PLACEHOLDER: PortalPullRequest[] = [
  ['Add retries to the checkout client', 'example-org/storefront', 1201, 4, 120, 18],
  ['Bump dependencies', 'example-org/order-service', 877, 1, 12, 12],
  ['Paginate the catalog endpoint', 'example-org/catalog-service', 342, 6, 230, 61],
  ['Fix flaky inventory test', 'example-org/inventory-service', 95, 0, 8, 3],
].map(([title, repo, number, comments, additions, deletions], i) => ({
  group: 'needs_review',
  title: title as string,
  repo: repo as string,
  number: number as number,
  url: '',
  author: { login: `member-${i}`, avatarUrl: '' },
  comments: comments as number,
  reviewers: [{ login: `reviewer-${i}`, avatarUrl: '' }],
  additions: additions as number,
  deletions: deletions as number,
  updatedAt: Date.now() - (i + 1) * 3_600_000,
}));

function PrHeader() {
  return (
    <div className="ph-pr-row ph-table-head">
      <span>Name</span>
      <span>Comments</span>
      <span>Reviewers</span>
      <span>Changes</span>
      <span>Updated</span>
    </div>
  );
}

function PrRow({ pr }: { pr: PortalPullRequest }) {
  const body = (
    <>
      <span className="ph-pr-name">
        <UserAvatar id={pr.author.login} name={pr.author.login} size={22} />
        <span className="ph-pr-text">
          <span className="ph-pr-title">{pr.title}</span>
          <span className="ph-pr-repo">
            {pr.repo} · #{pr.number}
          </span>
        </span>
      </span>
      <span className="ph-pr-comments">
        <MessageSquare size={13} /> {pr.comments}
      </span>
      <span className="ph-avatars">
        {pr.reviewers.slice(0, 4).map((r) => (
          <UserAvatar key={r.login} id={r.login} name={r.login} size={20} />
        ))}
        {pr.reviewers.length > 4 && <span className="ph-more">+{pr.reviewers.length - 4}</span>}
        {pr.reviewers.length === 0 && <span className="sc-muted">—</span>}
      </span>
      <span className="ph-changes">
        {pr.additions == null ? (
          <span className="sc-muted">—</span>
        ) : (
          <>
            <span className="ph-add">+{pr.additions}</span>
            <span className="ph-del">−{pr.deletions ?? 0}</span>
          </>
        )}
      </span>
      <span className="sc-muted">{age(pr.updatedAt)}</span>
    </>
  );
  return pr.url ? (
    <a className="ph-pr-row ph-row" href={pr.url} target="_blank" rel="noreferrer noopener">
      {body}
    </a>
  ) : (
    <div className="ph-pr-row ph-row">{body}</div>
  );
}

function PlaceholderTable() {
  return (
    <>
      <PrHeader />
      {PLACEHOLDER.map((p) => (
        <PrRow key={p.number} pr={p} />
      ))}
    </>
  );
}

function ProviderBody({ provider }: { provider: CodeHost }) {
  const navigate = useNavigate();
  const q = usePullRequests(provider);
  const label = provider === 'github' ? 'GitHub' : 'GitLab';
  const configure = () => navigate('/portal/settings/configurations');
  if (q.isLoading) return <Loading />;
  if (q.error) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const data = q.data!;
  if (data.status === 'not_connected' || data.status === 'error') {
    return (
      <ConnectOverlay
        logo={provider}
        name={label}
        title={`Could not connect to ${label}`}
        text={data.status === 'error' ? data.message : `Connect ${label} to see your pull requests`}
        action={`Configure ${label} Integration`}
        onAction={configure}
      >
        <PlaceholderTable />
      </ConnectOverlay>
    );
  }
  if (data.status === 'identity_required') {
    return (
      <ConnectOverlay logo={provider} name={label} title={`Who are you on ${label}?`} text={data.message} action={`Set your ${label} username`} onAction={configure}>
        <PlaceholderTable />
      </ConnectOverlay>
    );
  }
  const groups = groupPullRequests(data.pullRequests);
  return (
    <div className="ph-table">
      <PrHeader />
      {groups.map((g) => (
        <HomeGroup key={g.key} label={g.label} count={g.items.length} defaultOpen={g.key !== 'merged' && g.items.length > 0}>
          {g.items.length === 0 ? <div className="ph-group-empty">Nothing here</div> : g.items.map((pr) => <PrRow key={pr.url} pr={pr} />)}
        </HomeGroup>
      ))}
    </div>
  );
}

/** Home → Your PRs: the member's pull requests per code host. */
export function HomePullRequests() {
  const [tab, setTab] = useState<Tab>('github');
  const navigate = useNavigate();
  return (
    <HomeSection id="prs" icon={<GitPullRequest size={16} />} title="Your PRs">
      <ProviderTabs tabs={TABS} value={tab} onChange={setTab} />
      {tab === 'azure-devops' ? (
        <ConnectOverlay
          logo="tile:azure-devops"
          name="Azure DevOps"
          title="Azure DevOps is not supported yet"
          text="Pull requests from GitHub and GitLab can be shown today."
          action="Configure an Integration"
          onAction={() => navigate('/portal/settings/configurations')}
        >
          <PlaceholderTable />
        </ConnectOverlay>
      ) : (
        <ProviderBody key={tab} provider={tab} />
      )}
    </HomeSection>
  );
}
