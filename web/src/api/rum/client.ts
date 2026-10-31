// Real User Monitoring (Digital Experience → RUM): applications and the events of their SDKs. In "live"
// mode they come from the Rust API; otherwise the in-browser mock answers (mock/rum.ts).
//   GET    /api/v1/rum/applications           listRumApplications
//   POST   /api/v1/rum/applications           createRumApplication
//   GET    /api/v1/rum/applications/:id       getRumApplication
//   PUT    /api/v1/rum/applications/:id       updateRumApplication
//   DELETE /api/v1/rum/applications/:id       deleteRumApplication
//   POST   /api/v1/rum/events/search          searchRumEvents
//   POST   /api/v1/rum/facets                 rumFacets
//   POST   /api/v1/rum/summary                rumSummary
//   POST   /api/v1/product-analytics/report   productAnalyticsReport
//   POST   /api/v1/rum/replay/sessions/:id    getSessionReplay
//   PUT    /api/v1/rum/replay/sessions/:id/watched  markReplayWatched
//   GET    /api/v1/rum/replay/history         replayHistory
//   …      /api/v1/rum/replay/playlists[/:id[/sessions[/:session]]]  playlists
//   POST   /api/v1/rum/replay/heatmap         replayHeatmap
import { API_MODE, del, get, post, postTo, put } from '../http';
import * as mock from '../mock/rum';
import * as replayMock from '../mock/replay';
import type {
  AnalyticsReport,
  AnalyticsRequest,
  Heatmap,
  HeatmapRequest,
  PlaylistSession,
  ReplayPlaylist,
  ReplayPlaylistInput,
  SessionReplay,
  WatchedSession,
  RumApplication,
  RumApplicationInput,
  RumFacet,
  RumSearchPage,
  RumSearchRequest,
  RumSummary,
  RumSummaryRequest,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const listRumApplications = (): Promise<RumApplication[]> => (live ? get('/api/v1/rum/applications', {}) : mock.listRumApplications());
export const getRumApplication = (id: string): Promise<RumApplication> => (live ? get('/api/v1/rum/applications/{id}', { id }) : mock.getRumApplication(id));
export const createRumApplication = (input: RumApplicationInput): Promise<RumApplication> => (live ? post('/api/v1/rum/applications', input) : mock.createRumApplication(input));
export const updateRumApplication = (id: string, input: RumApplicationInput): Promise<RumApplication> =>
  live ? put('/api/v1/rum/applications/{id}', { id }, input) : mock.updateRumApplication(id, input);
export const deleteRumApplication = (id: string): Promise<void> => (live ? del('/api/v1/rum/applications/{id}', { id }) : mock.deleteRumApplication(id));
export const searchRumEvents = (req: RumSearchRequest, signal?: AbortSignal): Promise<RumSearchPage> => (live ? post('/api/v1/rum/events/search', req, signal) : mock.searchRumEvents(req));
export const rumFacets = (req: RumSearchRequest, signal?: AbortSignal): Promise<RumFacet[]> => (live ? post('/api/v1/rum/facets', req, signal) : mock.rumFacets(req));
export const rumSummary = (req: RumSummaryRequest, signal?: AbortSignal): Promise<RumSummary> => (live ? post('/api/v1/rum/summary', req, signal) : mock.rumSummary(req));
export const productAnalyticsReport = (req: AnalyticsRequest, signal?: AbortSignal): Promise<AnalyticsReport> =>
  live ? post('/api/v1/product-analytics/report', req, signal) : mock.productAnalyticsReport(req);

// Session Replay.
export const getSessionReplay = (id: string, startedAt: number, signal?: AbortSignal): Promise<SessionReplay> =>
  live ? postTo('/api/v1/rum/replay/sessions/{id}', { id }, { startedAt }, signal) : replayMock.getSessionReplay(id, startedAt);
export const markReplayWatched = (id: string, startedAt: number): Promise<void> =>
  live ? put('/api/v1/rum/replay/sessions/{id}/watched', { id }, { startedAt }) : replayMock.markReplayWatched(id, startedAt);
export const replayHistory = (): Promise<WatchedSession[]> => (live ? get('/api/v1/rum/replay/history', {}) : replayMock.replayHistory());
export const listReplayPlaylists = (): Promise<ReplayPlaylist[]> => (live ? get('/api/v1/rum/replay/playlists', {}) : replayMock.listReplayPlaylists());
export const getReplayPlaylist = (id: string): Promise<ReplayPlaylist> => (live ? get('/api/v1/rum/replay/playlists/{id}', { id }) : replayMock.getReplayPlaylist(id));
export const createReplayPlaylist = (input: ReplayPlaylistInput): Promise<ReplayPlaylist> => (live ? post('/api/v1/rum/replay/playlists', input) : replayMock.createReplayPlaylist(input));
export const updateReplayPlaylist = (id: string, input: ReplayPlaylistInput): Promise<ReplayPlaylist> =>
  live ? put('/api/v1/rum/replay/playlists/{id}', { id }, input) : replayMock.updateReplayPlaylist(id, input);
export const deleteReplayPlaylist = (id: string): Promise<void> => (live ? del('/api/v1/rum/replay/playlists/{id}', { id }) : replayMock.deleteReplayPlaylist(id));
export const addPlaylistSession = (id: string, s: PlaylistSession): Promise<ReplayPlaylist> =>
  live ? postTo('/api/v1/rum/replay/playlists/{id}/sessions', { id }, s) : replayMock.addPlaylistSession(id, s);
export const removePlaylistSession = (id: string, sessionId: string): Promise<void> =>
  live ? del('/api/v1/rum/replay/playlists/{id}/sessions/{session_id}', { id, session_id: sessionId }) : replayMock.removePlaylistSession(id, sessionId);
export const replayHeatmap = (req: HeatmapRequest, signal?: AbortSignal): Promise<Heatmap> => (live ? post('/api/v1/rum/replay/heatmap', req, signal) : replayMock.replayHeatmap(req));
