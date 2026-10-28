import { useParams } from 'react-router-dom';
import { ApiError } from '../../api/http';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { usePublicStatusPage } from './queries';
import { StatusPageView } from './StatusPageView';

/**
 * The read-only status page at /status/<slug>, outside the app's sign-in: public pages for anyone,
 * internal pages for signed-in members of the page's organization.
 */
export function PublicStatusPage() {
  const { slug = '' } = useParams();
  const q = usePublicStatusPage(slug);

  if (q.isPending) return <Loading />;
  if (q.isError && !(q.error instanceof ApiError && q.error.status === 404)) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <div className="stp-public">
      <main className="stp-public-main">
        {q.data ? (
          <StatusPageView page={q.data} />
        ) : (
          <>
            <h1 className="stp-public-title">Status page not found</h1>
            <p className="muted">
              This page doesn&apos;t exist or isn&apos;t public. If it&apos;s an internal page of your organization, <a href={`/login?redirect=${encodeURIComponent(`/status/${slug}`)}`}>sign in</a> to
              see it.
            </p>
          </>
        )}
      </main>
    </div>
  );
}
