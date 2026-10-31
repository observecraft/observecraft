import { useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ArrowLeft, History, ListVideo, Pencil, Plus, Search, Trash2, X } from 'lucide-react';
import type { PlaylistSession, ReplayPlaylist } from '../../../api/rum/client';
import { Modal } from '../../../components/ui/Modal';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { useDeletePlaylist, usePlaylistSession, useReplayHistory, useReplayPlaylist, useReplayPlaylists, useRumEvents, useSavePlaylist } from '../queries';
import { attr, RecordingsTable } from './common';

/** Playlists: the viewer's watch history and the saved collections of recordings. */
export function PlaylistsPage() {
  const playlists = useReplayPlaylists();
  const history = useReplayHistory();
  const [mine, setMine] = useState(false);
  const [search, setSearch] = useState('');
  const [editing, setEditing] = useState<ReplayPlaylist | 'new' | null>(null);
  const shown = (playlists.data ?? []).filter((p) => (!mine || p.createdByMe) && p.name.toLowerCase().includes(search.trim().toLowerCase()));
  return (
    <div className="replay-page">
      <div className="replay-toolbar">
        <h2 className="replay-h">Playlists</h2>
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> New Playlist
        </button>
      </div>
      <div className="replay-playlist-grid">
        <Link className="replay-playlist-card history" to="/rum/replay/playlists/history">
          <History size={22} />
          <b>My Watch History</b>
          <span className="muted">{history.data ? `${history.data.length} recording${history.data.length === 1 ? '' : 's'} watched` : '…'}</span>
        </Link>
      </div>
      <div className="replay-toolbar">
        <div className="replay-seg">
          <button className={!mine ? 'active' : ''} onClick={() => setMine(false)}>
            All playlists
          </button>
          <button className={mine ? 'active' : ''} onClick={() => setMine(true)}>
            Created by me
          </button>
        </div>
        <label className="replay-search">
          <Search size={14} />
          <input placeholder="Search playlists" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
      </div>
      {playlists.isLoading ? (
        <Loading />
      ) : playlists.isError ? (
        <ErrorState error={playlists.error} onRetry={() => playlists.refetch()} />
      ) : !shown.length ? (
        <Empty>
          <b>No playlists{mine ? ' of yours' : ''} yet.</b>
          <p className="muted">Collect recordings worth watching again, such as checkout errors or rage clicks, in a playlist.</p>
        </Empty>
      ) : (
        <div className="replay-playlist-grid">
          {shown.map((p) => (
            <Link key={p.id} className="replay-playlist-card" to={`/rum/replay/playlists/${p.id}`}>
              <ListVideo size={22} />
              <b>{p.name}</b>
              {p.description && <span className="replay-playlist-desc">{p.description}</span>}
              <span className="muted">
                {p.sessionCount} recording{p.sessionCount === 1 ? '' : 's'} · updated {timeAgo(p.updatedAt)}
              </span>
              <span className="muted">by {p.createdByMe ? 'me' : (p.createdBy ?? 'unknown')}</span>
            </Link>
          ))}
        </div>
      )}
      {editing && <PlaylistModal playlist={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function PlaylistModal({ playlist, onClose }: { playlist: ReplayPlaylist | null; onClose: () => void }) {
  const navigate = useNavigate();
  const save = useSavePlaylist(playlist?.id);
  const [name, setName] = useState(playlist?.name ?? '');
  const [description, setDescription] = useState(playlist?.description ?? '');
  return (
    <Modal onClose={onClose} label={playlist ? 'Edit playlist' : 'New playlist'} width={520} height="auto">
      <form
        className="replay-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({ name, description }, { onSuccess: (p) => (onClose(), playlist || navigate(`/rum/replay/playlists/${p.id}`)) });
        }}
      >
        <header>
          <h3>{playlist ? 'Edit playlist' : 'New playlist'}</h3>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <label>
          Name
          <input autoFocus required maxLength={200} value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label>
          Description
          <textarea rows={3} value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        {save.isError && <p className="replay-bad">{(save.error as Error).message}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!name.trim() || save.isPending}>
            {playlist ? 'Save' : 'Create'}
          </button>
        </footer>
      </form>
    </Modal>
  );
}

/** One playlist's recordings, or the viewer's watch history (`history`). */
export function PlaylistPage() {
  const { id } = useParams();
  const isHistory = id === 'history';
  const playlist = useReplayPlaylist(isHistory ? undefined : id);
  const history = useReplayHistory();
  const remove = usePlaylistSession();
  const del = useDeletePlaylist();
  const navigate = useNavigate();
  const [editing, setEditing] = useState(false);
  const entries: PlaylistSession[] | undefined = isHistory ? history.data : playlist.data?.sessions;
  const loading = isHistory ? history.isLoading : playlist.isLoading;
  const error = isHistory ? history.error : playlist.error;
  return (
    <div className="replay-page">
      <div className="replay-toolbar">
        <Link to="/rum/replay/playlists" className="btn btn-ghost" title="Back to the playlists">
          <ArrowLeft size={16} />
        </Link>
        <div className="replay-playlist-head">
          <h2 className="replay-h">{isHistory ? 'My Watch History' : (playlist.data?.name ?? 'Playlist')}</h2>
          {playlist.data?.description && <span className="muted">{playlist.data.description}</span>}
        </div>
        {playlist.data && (
          <>
            <button className="btn" onClick={() => setEditing(true)}>
              <Pencil size={14} /> Edit
            </button>
            <button className="btn" onClick={() => confirm(`Delete the playlist "${playlist.data!.name}"?`) && del.mutate(playlist.data!.id, { onSuccess: () => navigate('/rum/replay/playlists') })}>
              <Trash2 size={14} /> Delete
            </button>
          </>
        )}
      </div>
      {loading ? (
        <Loading />
      ) : error ? (
        <ErrorState error={error} />
      ) : !entries?.length ? (
        <Empty>
          <b>{isHistory ? 'You haven’t watched a recording yet.' : 'This playlist is empty.'}</b>
          <p className="muted">{isHistory ? 'Recordings you watch show up here.' : 'Add recordings to it from the player with “Add to playlist”.'}</p>
        </Empty>
      ) : (
        <PlaylistRecordings entries={entries} onRemove={isHistory ? undefined : (s) => remove.mutate({ playlist: id!, session: s, remove: true })} />
      )}
      {editing && playlist.data && <PlaylistModal playlist={playlist.data} onClose={() => setEditing(false)} />}
    </div>
  );
}

/** The sessions of a playlist, looked up in the time span they started in. */
function PlaylistRecordings({ entries, onRemove }: { entries: PlaylistSession[]; onRemove?: (s: PlaylistSession) => void }) {
  const range = useMemo(() => {
    const starts = entries.map((e) => e.startedAt);
    return { from: Math.min(...starts) - 3_600_000, to: Math.max(Date.now(), Math.max(...starts) + 4 * 3_600_000) };
  }, [entries]);
  const ids = entries.map((e) => e.sessionId);
  const list = useRumEvents({ range, query: '', eventType: 'session', applicationId: null, limit: 200, sessions: { ids, withReplay: true } });
  if (list.isLoading) return <Loading />;
  if (list.isError) return <ErrorState error={list.error} />;
  const rows = list.data?.rows ?? [];
  const order = new Map(ids.map((s, i) => [s, i]));
  rows.sort((a, b) => (order.get(attr(a, 'session.id')) ?? 0) - (order.get(attr(b, 'session.id')) ?? 0));
  const missing = ids.length - rows.length;
  return (
    <>
      <RecordingsTable
        rows={rows}
        actions={
          onRemove &&
          ((r) => (
            <button className="btn btn-ghost" title="Remove from the playlist" onClick={() => onRemove({ sessionId: attr(r, 'session.id'), startedAt: r.timestamp })}>
              <Trash2 size={14} />
            </button>
          ))
        }
      />
      {missing > 0 && (
        <p className="muted replay-count">
          {missing} recording{missing === 1 ? ' is' : 's are'} no longer retained.
        </p>
      )}
    </>
  );
}
