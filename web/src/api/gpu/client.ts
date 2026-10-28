// GPU Monitoring data access. In "live" mode GPUs come from the Rust API (the Datadog Agent's GPU and
// DCGM checks, or the DCGM exporter through an OpenTelemetry Collector, in ClickHouse); otherwise the
// in-browser mock answers.
//   POST /api/v1/gpu/search       searchGpus     (devices with current values, summary, facets)
//   POST /api/v1/gpu/timeseries   gpuTimeseries  (fleet utilization, memory, temperature, power)
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/gpu';

const live = API_MODE === 'live';

export type GpuDevice = components['schemas']['GpuDevice'];
export type GpuState = components['schemas']['GpuState'];
export type GpuSource = components['schemas']['GpuSource'];
export type GpuSummary = components['schemas']['GpuSummary'];
export type GpuFacet = components['schemas']['GpuFacet'];
export type GpuSearchPage = components['schemas']['GpuSearchPage'];
export type GpuSearchRequest = components['schemas']['GpuSearchRequest'];
export type GpuSeries = components['schemas']['GpuSeries'];
export type GpuTimeseriesRequest = components['schemas']['GpuTimeseriesRequest'];

export async function searchGpus(req: GpuSearchRequest): Promise<GpuSearchPage> {
  if (!live) return mock.searchGpus(req);
  return post('/api/v1/gpu/search', req);
}

export async function gpuTimeseries(req: GpuTimeseriesRequest): Promise<GpuSeries[]> {
  if (!live) return mock.gpuTimeseries(req);
  return post('/api/v1/gpu/timeseries', req);
}
