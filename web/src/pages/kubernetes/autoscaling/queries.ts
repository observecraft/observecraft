// TanStack Query bindings for Kubernetes → Autoscaling.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createAutoscalingProfile,
  deleteAutoscalingProfile,
  getAutoscaling,
  updateAutoscalingProfile,
  updateAutoscalingSettings,
  type AutoscalingSettings,
  type ProfileInput,
} from '../../../api/kubernetes/client';

const key = ['kubernetes', 'autoscaling'] as const;

/** Everything the Autoscaling pages show, refreshed every minute. */
export function useAutoscaling() {
  return useQuery({ queryKey: key, queryFn: getAutoscaling, refetchInterval: 60_000 });
}

export function useAutoscalingMutations() {
  const qc = useQueryClient();
  const done = () => qc.invalidateQueries({ queryKey: key });
  return {
    settings: useMutation({ mutationFn: (s: AutoscalingSettings) => updateAutoscalingSettings(s), onSuccess: done }),
    create: useMutation({ mutationFn: (p: ProfileInput) => createAutoscalingProfile(p), onSuccess: done }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: ProfileInput }) => updateAutoscalingProfile(id, input), onSuccess: done }),
    remove: useMutation({ mutationFn: (id: string) => deleteAutoscalingProfile(id), onSuccess: done }),
  };
}
