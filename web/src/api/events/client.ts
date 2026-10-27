// Event Management data access. In "live" mode events come from the Rust API (events the Datadog
// Agent and the events API send through the intake, in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/events/search   searchEvents
//   POST /api/v1/events/fields   eventFields
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

/** Tag keys of the matching events, for the facet panel's "+ Add". */
export async function eventFields(req: { range: EventSearchRequest['range']; query?: string }): Promise<{ key: string; count: number }[]> {
  if (!live) return mock.eventFields(req);
  return post('/api/v1/events/fields', { range: req.range, query: req.query ?? '' });
}
