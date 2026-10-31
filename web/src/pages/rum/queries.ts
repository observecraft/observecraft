// TanStack Query bindings for Real User Monitoring.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createRumApplication,
  deleteRumApplication,
  getRumApplication,
  listRumApplications,
  rumFacets,
  rumSummary,
  searchRumEvents,
  updateRumApplication,
  getSessionReplay,
  markReplayWatched,
  replayHistory,
  listReplayPlaylists,
  getReplayPlaylist,
  createReplayPlaylist,
  updateReplayPlaylist,
  deleteReplayPlaylist,
  addPlaylistSession,
  removePlaylistSession,
  replayHeatmap,
  type HeatmapRequest,
  type PlaylistSession,
  type ReplayPlaylistInput,
  type RumApplicationInput,
  type RumSearchRequest,
  type RumSummaryRequest,
} from '../../api/rum/client';

export const rumKeys = { all: ['rum'] as const };

export const useRumApplications = () => useQuery({ queryKey: [...rumKeys.all, 'apps'], queryFn: listRumApplications, staleTime: 30_000 });

export const useRumApplication = (id: string | undefined) => useQuery({ queryKey: [...rumKeys.all, 'app', id], queryFn: () => getRumApplication(id!), enabled: !!id });

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: rumKeys.all });
}

export function useSaveRumApplication(id?: string) {
  const done = useInvalidate();
  return useMutation({ mutationFn: (input: RumApplicationInput) => (id ? updateRumApplication(id, input) : createRumApplication(input)), onSuccess: done });
}

export function useDeleteRumApplication() {
  const done = useInvalidate();
  return useMutation({ mutationFn: deleteRumApplication, onSuccess: done });
}

export const useRumEvents = (req: RumSearchRequest) =>
  useQuery({ queryKey: [...rumKeys.all, 'events', req], queryFn: ({ signal }) => searchRumEvents(req, signal), placeholderData: keepPreviousData, retry: false, refetchInterval: 60_000 });

export const useRumFacets = (req: RumSearchRequest) =>
  useQuery({ queryKey: [...rumKeys.all, 'facets', req], queryFn: ({ signal }) => rumFacets(req, signal), placeholderData: keepPreviousData, retry: false });

export const useRumSummary = (req: RumSummaryRequest | null) =>
  useQuery({ queryKey: [...rumKeys.all, 'summary', req], queryFn: ({ signal }) => rumSummary(req!, signal), enabled: !!req, placeholderData: keepPreviousData, refetchInterval: 60_000 });

// Session Replay.
const replayKeys = [...rumKeys.all, 'replay'] as const;

export const useSessionReplay = (id: string | undefined, startedAt: number) =>
  useQuery({ queryKey: [...replayKeys, 'session', id, startedAt], queryFn: ({ signal }) => getSessionReplay(id!, startedAt, signal), enabled: !!id, retry: false, staleTime: 60_000 });

export function useMarkWatched() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, startedAt }: PlaylistSession & { id: string }) => markReplayWatched(id, startedAt),
    onSuccess: () => qc.invalidateQueries({ queryKey: [...replayKeys, 'history'] }),
  });
}

export const useReplayHistory = () => useQuery({ queryKey: [...replayKeys, 'history'], queryFn: replayHistory });

export const useReplayPlaylists = () => useQuery({ queryKey: [...replayKeys, 'playlists'], queryFn: listReplayPlaylists });

export const useReplayPlaylist = (id: string | undefined) => useQuery({ queryKey: [...replayKeys, 'playlist', id], queryFn: () => getReplayPlaylist(id!), enabled: !!id });

function useInvalidatePlaylists() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: replayKeys });
}

export function useSavePlaylist(id?: string) {
  const done = useInvalidatePlaylists();
  return useMutation({ mutationFn: (input: ReplayPlaylistInput) => (id ? updateReplayPlaylist(id, input) : createReplayPlaylist(input)), onSuccess: done });
}

export function useDeletePlaylist() {
  const done = useInvalidatePlaylists();
  return useMutation({ mutationFn: deleteReplayPlaylist, onSuccess: done });
}

export function usePlaylistSession() {
  const done = useInvalidatePlaylists();
  return useMutation({
    mutationFn: ({ playlist, session, remove }: { playlist: string; session: PlaylistSession; remove?: boolean }) =>
      remove ? removePlaylistSession(playlist, session.sessionId) : addPlaylistSession(playlist, session).then(() => undefined),
    onSuccess: done,
  });
}

export const useReplayHeatmap = (req: HeatmapRequest) =>
  useQuery({ queryKey: [...replayKeys, 'heatmap', req], queryFn: ({ signal }) => replayHeatmap(req, signal), placeholderData: keepPreviousData, retry: false });
