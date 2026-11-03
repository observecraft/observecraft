// TanStack Query bindings for Assist.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  assistReport,
  deleteAssistProvider,
  deleteConversation,
  getAssistProvider,
  getAssistSettings,
  getAssistStatus,
  getConversation,
  listConversations,
  putAssistProvider,
  testAssistProvider,
  updateAssistSettings,
  type AssistReportRequest,
} from '../../api/assist/client';

export const assistKeys = { all: ['assist'] as const, conversations: ['assist', 'conversations'] as const };

export const useAssistStatus = () => useQuery({ queryKey: [...assistKeys.all, 'status'], queryFn: getAssistStatus, staleTime: 60_000 });
export const useAssistSettings = () => useQuery({ queryKey: [...assistKeys.all, 'settings'], queryFn: getAssistSettings });
export const useConversations = () => useQuery({ queryKey: assistKeys.conversations, queryFn: listConversations });
export const useConversation = (id: string | undefined) =>
  useQuery({ queryKey: [...assistKeys.conversations, id], queryFn: ({ signal }) => getConversation(id!, signal), enabled: !!id, staleTime: Infinity });
export const useAssistReport = (req: AssistReportRequest) =>
  useQuery({ queryKey: [...assistKeys.all, 'report', req], queryFn: ({ signal }) => assistReport(req, signal), placeholderData: keepPreviousData });

export function useUpdateAssistSettings() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: updateAssistSettings, onSuccess: () => qc.invalidateQueries({ queryKey: [...assistKeys.all, 'settings'] }) });
}

export function useDeleteConversation() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deleteConversation, onSuccess: () => qc.invalidateQueries({ queryKey: assistKeys.conversations }) });
}

export const useAssistProvider = () => useQuery({ queryKey: [...assistKeys.all, 'provider'], queryFn: getAssistProvider });

/** Saving or removing the organization's provider changes what every page's status shows. */
export function useSaveAssistProvider() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: putAssistProvider, onSuccess: () => qc.invalidateQueries({ queryKey: assistKeys.all }) });
}

export function useDeleteAssistProvider() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: deleteAssistProvider, onSuccess: () => qc.invalidateQueries({ queryKey: assistKeys.all }) });
}

export const useTestAssistProvider = () => useMutation({ mutationFn: testAssistProvider });
