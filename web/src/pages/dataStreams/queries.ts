import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { kafkaConsole, streamMap, streamMeasure, streamNode } from '../../api/dataStreams/client';
import { useTime } from '../../components/shell/TimeContext';

/** The map of the selected time range (shared by Map, Pipelines and Measure). */
export function useStreamMap() {
  const { range } = useTime();
  return useQuery({ queryKey: ['data-streams', 'map', range], queryFn: () => streamMap({ range }), placeholderData: keepPreviousData });
}

export function useStreamNode(node: string | null) {
  const { range } = useTime();
  return useQuery({ queryKey: ['data-streams', 'node', range, node], queryFn: () => streamNode({ range, node: node! }), enabled: !!node, placeholderData: keepPreviousData });
}

export function useStreamMeasure(start: string | null, end: string | null) {
  const { range } = useTime();
  return useQuery({
    queryKey: ['data-streams', 'measure', range, start, end],
    queryFn: () => streamMeasure({ range, start: start!, end: end! }),
    enabled: !!start && !!end && start !== end,
    placeholderData: keepPreviousData,
  });
}

export function useKafkaConsole() {
  const { range } = useTime();
  return useQuery({ queryKey: ['data-streams', 'kafka', range], queryFn: () => kafkaConsole({ range }), placeholderData: keepPreviousData });
}
