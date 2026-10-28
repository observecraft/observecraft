// TanStack Query bindings for profile, users, roles, teams and SAML mappings.
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createMapping,
  createTeam,
  getProfile,
  inviteUsers,
  listMappings,
  listRoles,
  searchTeams,
  searchUsers,
  updateProfile,
  updateUser,
  type MappingKind,
  type TeamSearchRequest,
  type UserSearchRequest,
} from '../../api/identity/client';

export const idKeys = {
  all: ['identity'] as const,
  profile: () => [...idKeys.all, 'profile'] as const,
  users: (req: UserSearchRequest) => [...idKeys.all, 'users', req] as const,
  roles: () => [...idKeys.all, 'roles'] as const,
  teams: (req: TeamSearchRequest) => [...idKeys.all, 'teams', req] as const,
  mappings: (kind: MappingKind) => [...idKeys.all, 'mappings', kind] as const,
};

export const useProfile = () => useQuery({ queryKey: idKeys.profile(), queryFn: getProfile, staleTime: 60_000, retry: 1 });

/** What the signed-in user's role allows: Standard and Admin edit content; only Admin org-wide controls. */
export function useAccess(): { canWrite: boolean; isAdmin: boolean } {
  const me = useProfile();
  const org = me.data?.org.id;
  const has = (role: string) => me.data?.roles.some((r) => r.id === `${org}-${role}`) ?? false;
  const isAdmin = has('admin');
  return { canWrite: isAdmin || has('standard'), isAdmin };
}
export const useUsers = (req: UserSearchRequest) => useQuery({ queryKey: idKeys.users(req), queryFn: () => searchUsers(req), placeholderData: keepPreviousData, retry: 1 });
export const useRoles = () => useQuery({ queryKey: idKeys.roles(), queryFn: listRoles, staleTime: 60_000, retry: 1 });
export const useTeams = (req: TeamSearchRequest) => useQuery({ queryKey: idKeys.teams(req), queryFn: () => searchTeams(req), placeholderData: keepPreviousData, retry: 1 });
export const useMappings = (kind: MappingKind) => useQuery({ queryKey: idKeys.mappings(kind), queryFn: () => listMappings(kind), retry: 1 });

/** Every identity mutation invalidates all identity queries: the data sets are small. */
function useIdentityMutation<A>(fn: (a: A) => Promise<unknown>) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSuccess: () => qc.invalidateQueries({ queryKey: idKeys.all }) });
}

export const useUpdateProfile = () => useIdentityMutation(updateProfile);
export const useInviteUsers = () => useIdentityMutation(({ emails, roleIds }: { emails: string[]; roleIds: string[] }) => inviteUsers(emails, roleIds));
export const useUpdateUser = () => useIdentityMutation(({ id, ...p }: { id: string; status?: 'active' | 'pending' | 'deactivated'; roleIds?: string[] }) => updateUser(id, p));
export const useCreateTeam = () => useIdentityMutation(createTeam);
export const useCreateMapping = () => useIdentityMutation(createMapping);
