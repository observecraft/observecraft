import { ChevronDown, Search } from 'lucide-react';
import { Select } from '../../components/ui/Dropdown';
import { useCategories } from './queries';

/** Search box shared by the catalog and the marketplace. */
export function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <label className="int-search">
      <Search size={17} />
      <input value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} autoFocus />
    </label>
  );
}

/** Featured category pills plus an "All Categories" dropdown. Single selection; click again to clear. */
export function CategoryChips({ value, onChange }: { value: string | null; onChange: (c: string | null) => void }) {
  const cats = useCategories();
  const featured = cats.data?.featured ?? [];
  const all = cats.data?.all ?? [];
  const extra = value && !featured.includes(value) ? value : null;
  return (
    <div className="int-chips">
      {featured.map((c) => (
        <button key={c} className={`int-chip${value === c ? ' active' : ''}`} onClick={() => onChange(value === c ? null : c)}>
          {c}
        </button>
      ))}
      <Select
        value={value ?? ''}
        onChange={(v) => onChange(v || null)}
        options={[{ value: '', label: 'All Categories' }, ...all.map((c) => ({ value: c, label: c }))]}
        triggerClassName={`int-chip int-chip-all${extra ? ' active' : ''}`}
        trigger={() => (
          <>
            {extra ?? 'All Categories'} <ChevronDown size={15} />
          </>
        )}
      />
    </div>
  );
}
