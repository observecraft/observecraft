// Fleet Automation (Integrations → Fleet Automation). In "live" mode the fleet comes from the Rust API
// (Datadog Agent host metadata and checks in Postgres, liveness and OpenTelemetry Collectors from
// ClickHouse); otherwise the in-browser mock answers (mock/fleet.ts). Read-only: no remote changes.
//   POST /api/v1/fleet/agents          searchFleet          (agents, summary cards, versions, facets)
//   POST /api/v1/fleet/agent           getFleetAgent        (checks, sanitised configuration, agent errors)
//   POST /api/v1/fleet/configurations  listFleetProfiles    (agents grouped by identical configuration)
import { API_MODE, post } from '../http';
import * as mock from '../mock/fleet';
import type { FleetAgentDetail, FleetAgentRequest, FleetConfigurationsRequest, FleetPage, FleetProfile, FleetSearchRequest } from './types';

export * from './types';

const live = API_MODE === 'live';

export const searchFleet = (req: FleetSearchRequest, signal?: AbortSignal): Promise<FleetPage> => (live ? post('/api/v1/fleet/agents', req, signal) : mock.searchFleet(req));
export const getFleetAgent = (req: FleetAgentRequest, signal?: AbortSignal): Promise<FleetAgentDetail> => (live ? post('/api/v1/fleet/agent', req, signal) : mock.getFleetAgent(req));
export const listFleetProfiles = (req: FleetConfigurationsRequest, signal?: AbortSignal): Promise<FleetProfile[]> =>
  live ? post('/api/v1/fleet/configurations', req, signal) : mock.listFleetProfiles(req);
