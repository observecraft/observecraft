// TanStack Query bindings for incidents.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  addDocument,
  addFollowup,
  addImpact,
  addResponder,
  createNotification,
  createPage,
  generatePostmortem,
  getPostmortem,
  listDocuments,
  listNotifications,
  listPages,
  listResponders,
  removeDocument,
  removeResponder,
  savePostmortem,
  setImportant,
  updatePage,
  addNote,
  declareIncident,
  getIncident,
  getTimeline,
  searchIncidents,
  setFollowupDone,
  updateIncident,
  type DeclareRequest,
  type IncidentPatch,
  type IncidentSearchRequest,
} from '../../api/incidents/client';

export const incKeys = {
  all: ['incidents'] as const,
  search: (req: IncidentSearchRequest) => [...incKeys.all, 'search', req] as const,
  one: (id: number) => [...incKeys.all, 'one', id] as const,
  timeline: (id: number) => [...incKeys.all, 'timeline', id] as const,
};

export const useIncidents = (req: IncidentSearchRequest) => useQuery({ queryKey: incKeys.search(req), queryFn: () => searchIncidents(req), placeholderData: keepPreviousData, retry: 1 });
export const useIncident = (id: number) => useQuery({ queryKey: incKeys.one(id), queryFn: () => getIncident(id), retry: 1 });
export const useTimeline = (id: number) => useQuery({ queryKey: incKeys.timeline(id), queryFn: () => getTimeline(id), retry: 1 });
const sub = (id: number, name: string) => [...incKeys.all, name, id] as const;
export const useResponders = (id: number) => useQuery({ queryKey: sub(id, 'responders'), queryFn: () => listResponders(id), retry: 1 });
export const usePages = (id: number) => useQuery({ queryKey: sub(id, 'pages'), queryFn: () => listPages(id), retry: 1 });
export const useNotifications = (id: number) => useQuery({ queryKey: sub(id, 'notifications'), queryFn: () => listNotifications(id), retry: 1 });
export const useDocuments = (id: number) => useQuery({ queryKey: sub(id, 'documents'), queryFn: () => listDocuments(id), retry: 1 });
export const usePostmortem = (id: number) => useQuery({ queryKey: sub(id, 'postmortem'), queryFn: () => getPostmortem(id), retry: 1 });

function useIncidentMutation<A>(fn: (a: A) => Promise<unknown>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSuccess: () => qc.invalidateQueries({ queryKey: incKeys.all }) });
}

export const useDeclare = () => useIncidentMutation((req: DeclareRequest) => declareIncident(req));
export const useUpdateIncident = (id: number) => useIncidentMutation((p: IncidentPatch) => updateIncident(id, p));
export const useAddNote = (id: number) => useIncidentMutation((body: string) => addNote(id, body));
export const useAddImpact = (id: number) => useIncidentMutation(({ description, startAt }: { description: string; startAt: number }) => addImpact(id, description, startAt));
export const useAddFollowup = (id: number) => useIncidentMutation((title: string) => addFollowup(id, title));
export const useToggleFollowup = (id: number) => useIncidentMutation(({ followupId, done }: { followupId: number; done: boolean }) => setFollowupDone(id, followupId, done));

/** "1 day 8 hours 9 minutes 41 seconds" */
export function durationLong(ms: number): string {
  let s = Math.max(0, Math.round(ms / 1000));
  const parts: string[] = [];
  for (const [unit, size] of [
    ['day', 86400],
    ['hour', 3600],
    ['minute', 60],
    ['second', 1],
  ] as const) {
    const n = Math.floor(s / size);
    s -= n * size;
    if (n > 0) parts.push(`${n} ${unit}${n === 1 ? '' : 's'}`);
  }
  return parts.join(' ') || '0 seconds';
}

/** "12d", "3h", "25m" */
export function durationShort(ms: number): string {
  const m = Math.max(0, ms) / 60000;
  if (m < 60) return `${Math.max(1, Math.round(m))}m`;
  if (m < 60 * 24) return `${Math.round(m / 60)}h`;
  return `${Math.round(m / 1440)}d`;
}

export function fmtDateTime(ts: number, withYear = true): string {
  return new Date(ts)
    .toLocaleString('en-US', { month: 'short', day: 'numeric', year: withYear ? 'numeric' : undefined, hour: 'numeric', minute: '2-digit' })
    .replace(' AM', ' am')
    .replace(' PM', ' pm');
}

export const useSetImportant = (id: number) => useIncidentMutation(({ eventId, important }: { eventId: number; important: boolean }) => setImportant(id, eventId, important));
export const useAddResponder = (id: number) => useIncidentMutation(({ userId, role }: { userId: string; role: string }) => addResponder(id, userId, role));
export const useRemoveResponder = (id: number) => useIncidentMutation((userId: string) => removeResponder(id, userId));
export const useCreatePage = (id: number) => useIncidentMutation(({ team, title }: { team: string; title: string }) => createPage(id, team, title));
export const useUpdatePage = (id: number) => useIncidentMutation(({ pageId, status }: { pageId: number; status: 'acknowledged' | 'resolved' }) => updatePage(id, pageId, status));
export const useCreateNotification = (id: number) => useIncidentMutation(({ recipients, subject, body }: { recipients: string[]; subject: string; body: string }) => createNotification(id, recipients, subject, body));
export const useAddDocument = (id: number) => useIncidentMutation(({ title, url }: { title: string; url: string }) => addDocument(id, title, url));
export const useRemoveDocument = (id: number) => useIncidentMutation((docId: number) => removeDocument(id, docId));
export const useGeneratePostmortem = (id: number) => useIncidentMutation((template: 'standard' | 'brief') => generatePostmortem(id, template));
export const useSavePostmortem = (id: number) => useIncidentMutation((content: string) => savePostmortem(id, content));
