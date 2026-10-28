import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { getFleetAgent, listFleetProfiles, searchFleet } from '../../api/fleet/client';
import { useTime } from '../../components/shell/TimeContext';

/** The fleet in the page's time range; `query` filters the list (summary and facets stay fleet-wide). */
export function useFleet(query = '') {
  const { range } = useTime();
  return useQuery({ queryKey: ['fleet', 'agents', range, query], queryFn: ({ signal }) => searchFleet({ range, query }, signal), placeholderData: keepPreviousData });
}

export function useFleetAgent(id: string | null) {
  const { range } = useTime();
  return useQuery({ queryKey: ['fleet', 'agent', range, id], queryFn: ({ signal }) => getFleetAgent({ range, id: id! }, signal), enabled: id != null });
}

export function useFleetProfiles() {
  const { range } = useTime();
  return useQuery({ queryKey: ['fleet', 'profiles', range], queryFn: ({ signal }) => listFleetProfiles({ range }, signal), placeholderData: keepPreviousData });
}
