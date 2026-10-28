import { useEffect, useState } from 'react';
import { Search, Sparkles } from 'lucide-react';
import { ProfileMenu } from './ProfileMenu';
import { SearchPalette } from './SearchPalette';
import { ShowNavButton } from './Sidebar';

export function TopBar({ onShowNav }: { onShowNav?: () => void }) {
  const [searching, setSearching] = useState(false);
  // Ctrl+K / Cmd+K opens the search from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setSearching((s) => !s);
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);
  return (
    <header className="topbar">
      {onShowNav && <ShowNavButton onClick={onShowNav} />}
      <button type="button" className="topbar-search" onClick={() => setSearching(true)}>
        <Search size={14} />
        <span>Search ObserveCraft</span>
        <kbd>Ctrl</kbd>
        <kbd>K</kbd>
      </button>
      {searching && <SearchPalette onClose={() => setSearching(false)} />}
      <div className="topbar-right">
        <ProfileMenu />
        <button className="ask-btn">
          <Sparkles size={13} />
          Ask Assist
        </button>
      </div>
    </header>
  );
}
