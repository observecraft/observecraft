// Event Management data access. In "live" mode events come from the Rust API (events the Datadog
// Agent and the events API send through the intake, in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/events/search   searchEvents
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/events';

const live = API_MODE === 'live';

export type EventItem = components['schemas']['Event'];
export type EventPage = components['schemas']['EventPage'];
export type EventSearchRequest = components['schemas']['EventSearchRequest'];
export type EventBucket = components['schemas']['EventBucket'];

export async function searchEvents(req: EventSearchRequest): Promise<EventPage> {
  if (!live) return mock.searchEvents(req);
  return post('/api/v1/events/search', req);
}
