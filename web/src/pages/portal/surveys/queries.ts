// TanStack Query bindings for Developer Portal surveys.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createSurvey,
  deleteSurvey,
  getSurvey,
  getSurveyForm,
  getSurveyResults,
  listSurveyResponses,
  listSurveys,
  submitSurveyResponse,
  updateSurvey,
  type SurveyInput,
  type SurveyResponseInput,
} from '../../../api/portalSurveys/client';

export const surveyKeys = {
  all: ['portal-surveys'] as const,
  one: (id: string) => ['portal-surveys', id] as const,
  form: (id: string) => ['portal-surveys', id, 'form'] as const,
  results: (id: string) => ['portal-surveys', id, 'results'] as const,
};

export const useSurveys = () => useQuery({ queryKey: surveyKeys.all, queryFn: ({ signal }) => listSurveys(signal), retry: 1 });
export const useSurvey = (id: string | undefined) => useQuery({ queryKey: surveyKeys.one(id ?? ''), queryFn: ({ signal }) => getSurvey(id!, signal), enabled: !!id, retry: 1 });
export const useSurveyForm = (id: string) => useQuery({ queryKey: surveyKeys.form(id), queryFn: ({ signal }) => getSurveyForm(id, signal), retry: 1 });
export const useSurveyResults = (id: string) => useQuery({ queryKey: surveyKeys.results(id), queryFn: ({ signal }) => getSurveyResults(id, signal), retry: 1, refetchInterval: 60_000 });

function useSurveyMutation<V, R>(fn: (v: V) => Promise<R>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSuccess: () => qc.invalidateQueries({ queryKey: surveyKeys.all }) });
}

export const useCreateSurvey = () => useSurveyMutation((input: SurveyInput) => createSurvey(input));
export const useUpdateSurvey = () => useSurveyMutation(({ id, input }: { id: string; input: SurveyInput }) => updateSurvey(id, input));
export const useDeleteSurvey = () => useSurveyMutation((id: string) => deleteSurvey(id));
export const useSubmitSurveyResponse = () => useSurveyMutation(({ id, input }: { id: string; input: SurveyResponseInput }) => submitSurveyResponse(id, input));
/** Loads every response for the CSV export (on demand). */
export const useExportSurveyResponses = () => useMutation({ mutationFn: (id: string) => listSurveyResponses(id) });
