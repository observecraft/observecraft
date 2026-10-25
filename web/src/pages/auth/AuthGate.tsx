import { useEffect, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useLocation, useNavigate } from 'react-router-dom';
import { API_MODE, ApiError, UNAUTHORIZED_EVENT } from '../../api/http';
import { getProfile } from '../../api/identity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';

function toLogin(navigate: ReturnType<typeof useNavigate>, path: string) {
  navigate(`/login?redirect=${encodeURIComponent(path)}`, { replace: true });
}

/**
 * Renders the app only for a signed-in user (live mode). Any 401 later on — expired session,
 * "log out of all sessions" elsewhere, deactivation — returns to the sign-in page.
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const navigate = useNavigate();
  const location = useLocation();
  const qc = useQueryClient();
  const live = API_MODE === 'live';
  const me = useQuery({ queryKey: ['identity', 'profile'], queryFn: getProfile, enabled: live, retry: false, staleTime: 60_000 });
  const here = location.pathname + location.search;

  useEffect(() => {
    const onUnauthorized = () => {
      qc.clear();
      toLogin(navigate, window.location.pathname + window.location.search);
    };
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
  }, [navigate, qc]);

  const unauthorized = me.error instanceof ApiError && me.error.status === 401;
  useEffect(() => {
    if (unauthorized) toLogin(navigate, here);
  }, [unauthorized, navigate, here]);

  if (!live) return <>{children}</>;
  if (me.isPending || unauthorized) return <Loading label="Signing in…" />;
  if (me.isError) return <ErrorState error={me.error} onRetry={() => me.refetch()} />;
  return <>{children}</>;
}
