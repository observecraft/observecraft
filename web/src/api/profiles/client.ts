// Continuous Profiler data access. In "live" mode profiles come from the Rust API (Datadog profiler
// uploads through the intake, in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/profiles/overview          profileOverview   (metric over time, services, facets)
//   POST /api/v1/profiles/flamegraph        flameGraph        (merged flame graph, cards, endpoints, functions)
//   POST /api/v1/profiles/search            searchProfiles    (Profile List)
//   POST /api/v1/profiles/insights          profileInsights
//   POST /api/v1/profiles/insights/state    setInsightState
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/profiles';

const live = API_MODE === 'live';

export type ProfileMetric = components['schemas']['ProfileMetric'];
export type ProfileOverview = components['schemas']['ProfileOverview'];
export type ProfileOverviewRequest = components['schemas']['ProfileOverviewRequest'];
export type FlameGraph = components['schemas']['FlameGraph'];
export type FlameGraphRequest = components['schemas']['FlameGraphRequest'];
export type FlameNode = components['schemas']['FlameNode'];
export type ProfileCard = components['schemas']['ProfileCard'];
export type ProfileFunction = components['schemas']['ProfileFunction'];
export type ProfileEndpoint = components['schemas']['ProfileEndpoint'];
export type ProfileList = components['schemas']['ProfileList'];
export type ProfileSummary = components['schemas']['ProfileSummary'];
export type ProfileSearchRequest = components['schemas']['ProfileSearchRequest'];
export type ProfileInsight = components['schemas']['ProfileInsight'];
export type ProfileInsightList = components['schemas']['ProfileInsightList'];
export type ProfileInsightRequest = components['schemas']['ProfileInsightRequest'];
export type InsightStatus = 'for_review' | 'reviewed' | 'resolved' | 'ignored';

export async function profileOverview(req: ProfileOverviewRequest): Promise<ProfileOverview> {
  if (!live) return mock.profileOverview(req);
  return post('/api/v1/profiles/overview', req);
}

export async function flameGraph(req: FlameGraphRequest): Promise<FlameGraph> {
  if (!live) return mock.flameGraph(req);
  return post('/api/v1/profiles/flamegraph', req);
}

export async function searchProfiles(req: ProfileSearchRequest): Promise<ProfileList> {
  if (!live) return mock.searchProfiles(req);
  return post('/api/v1/profiles/search', req);
}

export async function profileInsights(req: ProfileInsightRequest): Promise<ProfileInsightList> {
  if (!live) return mock.profileInsights(req);
  return post('/api/v1/profiles/insights', req);
}

export async function setInsightState(key: string, status: InsightStatus): Promise<void> {
  if (!live) return mock.setInsightState(key, status);
  await post('/api/v1/profiles/insights/state', { key, status });
}
