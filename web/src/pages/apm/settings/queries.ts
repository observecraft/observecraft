// TanStack Query bindings for APM → Settings.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createRetentionFilter,
  createSpanPipeline,
  createSpansMetric,
  deleteRetentionFilter,
  deleteSpanPipeline,
  deleteSpansMetric,
  getApmDefaults,
  getApmFlow,
  getIngestion,
  getSpanVolume,
  listRetentionFilters,
  listSpanPipelines,
  listSpansMetrics,
  reorderRetentionFilters,
  updateApmDefaults,
  updateRetentionFilter,
  updateSpanPipeline,
  updateSpansMetric,
  type RetentionFilterInput,
  type SpanPipelineInput,
  type SpansMetricInput,
  type TimeRange,
} from '../../../api/apmSettings/client';

const keys = { all: ['apm-settings'] as const };

function useInvalidate() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: keys.all });
}

const ranged = <T>(name: string, range: TimeRange, fn: (r: TimeRange) => Promise<T>) =>
  ({ queryKey: [...keys.all, name, range.from, range.to], queryFn: () => fn(range), placeholderData: keepPreviousData }) as const;

export const useApmFlow = (range: TimeRange) => useQuery(ranged('overview', range, getApmFlow));
export const useSpanVolume = (range: TimeRange) => useQuery(ranged('volume', range, getSpanVolume));
export const useIngestion = (range: TimeRange) => useQuery(ranged('ingestion', range, getIngestion));
export const useRetentionFilters = (range: TimeRange) => useQuery(ranged('retention-filters', range, listRetentionFilters));
export const useApmDefaults = () => useQuery({ queryKey: [...keys.all, 'defaults'], queryFn: getApmDefaults });
export const useSpansMetrics = () => useQuery({ queryKey: [...keys.all, 'metrics'], queryFn: listSpansMetrics });
export const useSpanPipelines = () => useQuery({ queryKey: [...keys.all, 'pipelines'], queryFn: listSpanPipelines });

export function useUpdateDefaults() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: updateApmDefaults, onSuccess: invalidate });
}
export function useSaveRetentionFilter() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: ({ id, input }: { id?: string; input: RetentionFilterInput }) => (id ? updateRetentionFilter(id, input) : createRetentionFilter(input)), onSuccess: invalidate });
}
export function useDeleteRetentionFilter() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: deleteRetentionFilter, onSuccess: invalidate });
}
export function useReorderRetentionFilters() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: reorderRetentionFilters, onSuccess: invalidate });
}
export function useSaveSpansMetric() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: ({ id, input }: { id?: string; input: SpansMetricInput }) => (id ? updateSpansMetric(id, input) : createSpansMetric(input)), onSuccess: invalidate });
}
export function useDeleteSpansMetric() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: deleteSpansMetric, onSuccess: invalidate });
}
export function useSaveSpanPipeline() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: ({ id, input }: { id?: string; input: SpanPipelineInput }) => (id ? updateSpanPipeline(id, input) : createSpanPipeline(input)), onSuccess: invalidate });
}
export function useDeleteSpanPipeline() {
  const invalidate = useInvalidate();
  return useMutation({ mutationFn: deleteSpanPipeline, onSuccess: invalidate });
}
