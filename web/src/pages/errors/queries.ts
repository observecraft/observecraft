// TanStack Query bindings for Error Tracking. Components use these hooks, never the client directly,
// so caching, refetching and invalidation rules live in one place.
import { keepPreviousData, useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  addComment,
  getErrorSample,
  getIssue,
  issueFacets,
  listTeams,
  listUsers,
  searchIssues,
  updateIssue,
} from '../../api/errorTracking/client';
import type { ErrorSampleRequest, IssueFacetsRequest, IssuePatch, IssueSearchRequest } from '../../api/errorTracking/types';
import type { TimeRange } from '../../api/types';

export { snapRange } from '../../lib/time';

export const errorKeys = {
  all: ['error-tracking'] as const,
  issues: (req: Omit<IssueSearchRequest, 'cursor'>) => [...errorKeys.all, 'issues', req] as const,
  facets: (req: IssueFacetsRequest) => [...errorKeys.all, 'facets', req] as const,
  issue: (id: string, range: TimeRange) => [...errorKeys.all, 'issue', id, range] as const,
  sample: (req: ErrorSampleRequest) => [...errorKeys.all, 'sample', req] as const,
  users: () => [...errorKeys.all, 'users'] as const,
  teams: () => [...errorKeys.all, 'teams'] as const,
};

export function useIssues(req: Omit<IssueSearchRequest, 'cursor'>) {
  return useInfiniteQuery({
    queryKey: errorKeys.issues(req),
    queryFn: ({ pageParam }) => searchIssues({ ...req, cursor: pageParam }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    placeholderData: keepPreviousData,
  });
}

export function useIssueFacets(req: IssueFacetsRequest) {
  return useQuery({ queryKey: errorKeys.facets(req), queryFn: () => issueFacets(req), placeholderData: keepPreviousData });
}

export function useIssue(id: string, range: TimeRange) {
  return useQuery({ queryKey: errorKeys.issue(id, range), queryFn: () => getIssue(id, range), placeholderData: keepPreviousData });
}

export function useErrorSample(req: ErrorSampleRequest) {
  return useQuery({ queryKey: errorKeys.sample(req), queryFn: () => getErrorSample(req), placeholderData: keepPreviousData });
}

export function useUsers() {
  return useQuery({ queryKey: errorKeys.users(), queryFn: listUsers, staleTime: Infinity });
}

export function useTeams() {
  return useQuery({ queryKey: errorKeys.teams(), queryFn: listTeams, staleTime: Infinity });
}

/** Status / assignee / cause changes. Everything under error-tracking is refetched afterwards. */
export function useUpdateIssue() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, patch }: { id: string; patch: IssuePatch }) => updateIssue(id, patch),
    onSuccess: () => qc.invalidateQueries({ queryKey: errorKeys.all }),
  });
}

export function useAddComment(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (text: string) => addComment(id, text),
    onSuccess: () => qc.invalidateQueries({ queryKey: [...errorKeys.all, 'issue', id] }),
  });
}
