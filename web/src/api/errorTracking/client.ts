// Error Tracking data access. Each function maps 1:1 to a query-service endpoint:
//   POST /api/v1/error-tracking/issues/search      searchIssues
//   POST /api/v1/error-tracking/issues/facets      issueFacets
//   GET  /api/v1/error-tracking/issues/:id         getIssue
//   PATCH /api/v1/error-tracking/issues/:id        updateIssue
//   POST /api/v1/error-tracking/issues/:id/comments addComment
//   POST /api/v1/error-tracking/issues/:id/samples getErrorSample
//   GET  /api/v1/users                              listUsers
//   GET  /api/v1/teams                              listTeams
// Live: issues come from the server (error logs grouped into issues); the rest (issue detail, states,
// comments, samples) isn't on the server yet and fails with an explanation rather than showing
// example data as if it were real. Mock mode answers everything in the browser.
import { API_MODE, post, postTo } from '../http';
import * as mock from '../mock/errorTracking';
import type { TimeRange } from '../types';

function requireMock(): void {
  if (API_MODE === 'live')
    throw new Error('Error Tracking is not connected to your data yet. Errors from logs and traces will be grouped into issues here; until then this page shows nothing rather than examples.');
}
import type { ActivityEntry, ErrorSamplePage, ErrorSampleRequest, IssueDetail, IssueFacets, IssueFacetsRequest, IssuePatch, IssueSearchPage, IssueSearchRequest, QuerySuggestion, User } from './types';

export async function searchIssues(req: IssueSearchRequest): Promise<IssueSearchPage> {
  if (API_MODE !== 'live') return mock.searchIssues(req);
  // Issues from the organization's error logs (persona, assignee, team and cause filters come with saved issues).
  const page = await post('/api/v1/error-tracking/issues/search', { range: req.range, query: req.query, state: req.state, source: req.source, sort: req.sort, cursor: req.cursor, limit: req.limit });
  return page as unknown as IssueSearchPage;
}

export function issueFacets(req: IssueFacetsRequest): Promise<IssueFacets> {
  // Facets over real issues come next; until then live shows none.
  if (API_MODE === 'live') return Promise.resolve({ facets: [], totalAvailable: 0 });
  return mock.issueFacets(req);
}

export function suggestIssueQuery(range: TimeRange, partial: string): Promise<QuerySuggestion[]> {
  if (API_MODE === 'live') return Promise.resolve([]);
  return mock.suggest(range, partial);
}

export async function getIssue(id: string, range: TimeRange): Promise<IssueDetail> {
  if (API_MODE !== 'live') return mock.getIssue(id, range);
  const detail = await postTo('/api/v1/error-tracking/issues/{id}', { id }, { range, query: '' });
  return detail as unknown as IssueDetail;
}

export async function updateIssue(id: string, patch: IssuePatch): Promise<void> {
  if (API_MODE !== 'live') {
    await mock.updateIssue(id, patch);
    return;
  }
  if (patch.suspectedCause !== undefined) throw new Error('Setting the suspected cause comes next; status and assignee can be changed.');
  await postTo('/api/v1/error-tracking/issues/{id}/state', { id }, { status: patch.status, assigneeId: patch.assigneeId });
}

export function addComment(id: string, text: string): Promise<ActivityEntry> {
  requireMock();
  return mock.addComment(id, text);
}

export async function getErrorSample(req: ErrorSampleRequest): Promise<ErrorSamplePage> {
  if (API_MODE !== 'live') return mock.getErrorSample(req);
  // Samples are among the issue's newest (or oldest) 2,000 logs in range.
  const page = await postTo('/api/v1/error-tracking/issues/{id}/samples', { id: req.issueId }, { range: req.range, query: req.query, index: req.index, order: req.order });
  return page as unknown as ErrorSamplePage;
}

export async function listUsers(): Promise<User[]> {
  if (API_MODE !== 'live') return mock.listUsers();
  const page = await post('/api/v1/users/search', { query: '', limit: 500, offset: 0 });
  return page.users.map((u) => ({ id: u.id, name: u.name, email: u.email }));
}

export function listTeams(): Promise<string[]> {
  return mock.listTeams();
}
