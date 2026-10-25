// Error Tracking data access. Each function maps 1:1 to a query-service endpoint:
//   POST /api/v1/error-tracking/issues/search      searchIssues
//   POST /api/v1/error-tracking/issues/facets      issueFacets
//   GET  /api/v1/error-tracking/issues/:id         getIssue
//   PATCH /api/v1/error-tracking/issues/:id        updateIssue
//   POST /api/v1/error-tracking/issues/:id/comments addComment
//   POST /api/v1/error-tracking/issues/:id/samples getErrorSample
//   GET  /api/v1/users                              listUsers
//   GET  /api/v1/teams                              listTeams
// Today they resolve against the in-browser mock.
import * as mock from '../mock/errorTracking';
import type { TimeRange } from '../types';
import type {
  ActivityEntry,
  ErrorSamplePage,
  ErrorSampleRequest,
  IssueDetail,
  IssueFacets,
  IssueFacetsRequest,
  IssuePatch,
  IssueSearchPage,
  IssueSearchRequest,
  QuerySuggestion,
  User,
} from './types';

export function searchIssues(req: IssueSearchRequest): Promise<IssueSearchPage> {
  return mock.searchIssues(req);
}

export function issueFacets(req: IssueFacetsRequest): Promise<IssueFacets> {
  return mock.issueFacets(req);
}

export function suggestIssueQuery(range: TimeRange, partial: string): Promise<QuerySuggestion[]> {
  return mock.suggest(range, partial);
}

export function getIssue(id: string, range: TimeRange): Promise<IssueDetail> {
  return mock.getIssue(id, range);
}

export function updateIssue(id: string, patch: IssuePatch): Promise<IssueDetail> {
  return mock.updateIssue(id, patch);
}

export function addComment(id: string, text: string): Promise<ActivityEntry> {
  return mock.addComment(id, text);
}

export function getErrorSample(req: ErrorSampleRequest): Promise<ErrorSamplePage> {
  return mock.getErrorSample(req);
}

export function listUsers(): Promise<User[]> {
  return mock.listUsers();
}

export function listTeams(): Promise<string[]> {
  return mock.listTeams();
}
