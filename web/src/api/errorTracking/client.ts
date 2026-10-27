// Error Tracking data access. Each function maps 1:1 to a query-service endpoint:
//   POST /api/v1/error-tracking/issues/search      searchIssues
//   POST /api/v1/error-tracking/issues/facets      issueFacets
//   GET  /api/v1/error-tracking/issues/:id         getIssue
//   PATCH /api/v1/error-tracking/issues/:id        updateIssue
//   POST /api/v1/error-tracking/issues/:id/comments addComment
//   POST /api/v1/error-tracking/issues/:id/samples getErrorSample
//   GET  /api/v1/users                              listUsers
//   GET  /api/v1/teams                              listTeams
// The server has no Error Tracking yet: these resolve against the in-browser mock in mock mode, and in
// live mode fail with an explanation instead of showing example issues as if they were real.
import { API_MODE } from '../http';
import * as mock from '../mock/errorTracking';
import type { TimeRange } from '../types';

function requireMock(): void {
  if (API_MODE === 'live')
    throw new Error('Error Tracking is not connected to your data yet. Errors from logs and traces will be grouped into issues here; until then this page shows nothing rather than examples.');
}
import type { ActivityEntry, ErrorSamplePage, ErrorSampleRequest, IssueDetail, IssueFacets, IssueFacetsRequest, IssuePatch, IssueSearchPage, IssueSearchRequest, QuerySuggestion, User } from './types';

export function searchIssues(req: IssueSearchRequest): Promise<IssueSearchPage> {
  requireMock();
  return mock.searchIssues(req);
}

export function issueFacets(req: IssueFacetsRequest): Promise<IssueFacets> {
  requireMock();
  return mock.issueFacets(req);
}

export function suggestIssueQuery(range: TimeRange, partial: string): Promise<QuerySuggestion[]> {
  if (API_MODE === 'live') return Promise.resolve([]);
  return mock.suggest(range, partial);
}

export function getIssue(id: string, range: TimeRange): Promise<IssueDetail> {
  requireMock();
  return mock.getIssue(id, range);
}

export function updateIssue(id: string, patch: IssuePatch): Promise<IssueDetail> {
  requireMock();
  return mock.updateIssue(id, patch);
}

export function addComment(id: string, text: string): Promise<ActivityEntry> {
  requireMock();
  return mock.addComment(id, text);
}

export function getErrorSample(req: ErrorSampleRequest): Promise<ErrorSamplePage> {
  requireMock();
  return mock.getErrorSample(req);
}

export function listUsers(): Promise<User[]> {
  return mock.listUsers();
}

export function listTeams(): Promise<string[]> {
  return mock.listTeams();
}
