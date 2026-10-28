import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createGate,
  createPrRule,
  deleteGate,
  deletePrRule,
  evaluateGate,
  evaluatePr,
  getEvaluation,
  listEvaluations,
  listGates,
  listPrRules,
  updateGate,
  updatePrRule,
  type DeploymentGateInput,
  type PrGateRuleInput,
} from '../../api/deliveryGates/client';

const keys = {
  gates: ['delivery', 'gates'] as const,
  evaluations: ['delivery', 'gates', 'evaluations'] as const,
  evaluation: (id: string) => ['delivery', 'gates', 'evaluations', id] as const,
  prRules: ['delivery', 'pr-gates'] as const,
};

export const useGates = () => useQuery({ queryKey: keys.gates, queryFn: listGates });
export const useEvaluations = () => useQuery({ queryKey: keys.evaluations, queryFn: listEvaluations, refetchInterval: 30_000 });
export const useEvaluation = (id: string) => useQuery({ queryKey: keys.evaluation(id), queryFn: () => getEvaluation(id) });

/** Gate writes and evaluations refresh the gates and the evaluations. */
export function useGateMutations() {
  const qc = useQueryClient();
  const onSuccess = () => qc.invalidateQueries({ queryKey: keys.gates });
  return {
    save: useMutation({ mutationFn: ({ id, input }: { id?: string; input: DeploymentGateInput }) => (id ? updateGate(id, input) : createGate(input)), onSuccess }),
    remove: useMutation({ mutationFn: deleteGate, onSuccess }),
    evaluate: useMutation({ mutationFn: evaluateGate, onSuccess }),
  };
}

export const usePrRules = () => useQuery({ queryKey: keys.prRules, queryFn: listPrRules });

export function usePrRuleMutations() {
  const qc = useQueryClient();
  const onSuccess = () => qc.invalidateQueries({ queryKey: keys.prRules });
  return {
    save: useMutation({ mutationFn: ({ id, input }: { id?: string; input: PrGateRuleInput }) => (id ? updatePrRule(id, input) : createPrRule(input)), onSuccess }),
    remove: useMutation({ mutationFn: deletePrRule, onSuccess }),
    evaluate: useMutation({ mutationFn: evaluatePr }),
  };
}
