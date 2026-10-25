import { Fragment, memo } from 'react';
import { ArrowDownWideNarrow, ArrowUpNarrowWide, Plus } from 'lucide-react';
import type { HostSearchPage, HostSortField, HostSummary } from '../../api/infrastructure/types';
import { CloudIcon, CollectorIcon, GpuIcon, OsIcon, SoftwareIcon } from './hostIcons';

export type HostColumn = 'configurations' | 'software' | 'env' | 'service' | 'monitors' | 'cpu' | 'errors';
export const ALL_COLUMNS: { key: HostColumn; label: string }[] = [
  { key: 'configurations', label: 'Configurations' },
  { key: 'software', label: 'Software' },
  { key: 'env', label: 'Env' },
  { key: 'service', label: 'Service' },
  { key: 'monitors', label: 'Monitors' },
  { key: 'cpu', label: 'CPU Usage %' },
  { key: 'errors', label: 'Errors' },
];

interface Props {
  page: HostSearchPage;
  columns: Set<HostColumn>;
  sort: { field: HostSortField; dir: 'asc' | 'desc' };
  onSort: (field: HostSortField) => void;
  selected: string | null;
  onOpen: (name: string) => void;
}

function Monitors({ m }: { m: HostSummary['monitors'] }) {
  if (m.alert) return <span className="mon-pill mon-alert">{m.alert} ALERT</span>;
  if (m.warn) return <span className="mon-pill mon-warn">{m.warn} WARN</span>;
  if (m.ok) return <span className="mon-pill mon-ok">{m.ok} OK</span>;
  return (
    <button className="mon-add" title="Monitor creation is not available yet" onClick={(e) => e.stopPropagation()}>
      <Plus size={14} /> Add Monitor
    </button>
  );
}

const Row = memo(function Row({ h, columns, selected, onOpen }: { h: HostSummary; columns: Set<HostColumn>; selected: boolean; onOpen: (n: string) => void }) {
  return (
    <tr className={selected ? 'selected' : ''} onClick={() => onOpen(h.name)}>
      <td className="ht-host">{h.name}</td>
      {columns.has('configurations') && (
        <td>
          <span className="ht-icons">
            <CloudIcon cloud={h.cloud} size={15} />
            <OsIcon os={h.os} distro={h.distro} size={15} />
            {h.collectors.map((c) => (
              <CollectorIcon key={c} kind={c} size={15} />
            ))}
            {h.gpu && <GpuIcon size={15} />}
          </span>
        </td>
      )}
      {columns.has('software') && (
        <td>
          <span className="ht-icons">
            {h.software.map((s) => (
              <SoftwareIcon key={s} kind={s} size={15} />
            ))}
          </span>
        </td>
      )}
      {columns.has('env') && <td>{h.env ? <span className="tag-chip">{h.env}</span> : <span className="tag-chip">-</span>}</td>}
      {columns.has('service') && <td>{h.service ? <span className="tag-chip">{h.service}</span> : <span className="tag-chip">-</span>}</td>}
      {columns.has('monitors') && (
        <td className="ht-center">
          <Monitors m={h.monitors} />
        </td>
      )}
      {columns.has('cpu') && (
        <td className="ht-cpu">
          <span>{h.cpuPct}%</span>
          <span className="usage-bar usage-primary">
            <span style={{ width: `${Math.max(3, h.cpuPct)}%` }} />
          </span>
        </td>
      )}
      {columns.has('errors') && <td>{h.errorSpans ? <span className="err-pill">{h.errorSpans} {h.errorSpans === 1 ? 'SPAN' : 'SPANS'}</span> : null}</td>}
    </tr>
  );
});

export function HostTable({ page, columns, sort, onSort, selected, onOpen }: Props) {
  const SortIcon = sort.dir === 'desc' ? ArrowDownWideNarrow : ArrowUpNarrowWide;
  const th = (label: string, field?: HostSortField, className?: string) => (
    <th className={`${className ?? ''}${field ? ' sortable' : ''}`} onClick={field ? () => onSort(field) : undefined}>
      {field && sort.field === field && <SortIcon size={13} />} {label}
    </th>
  );
  const colCount = 1 + columns.size;
  let lastGroup: string | undefined;

  return (
    <table className="host-table">
      <thead>
        <tr>
          {th('HOST', 'name', 'ht-host')}
          {columns.has('configurations') && th('CONFIGURATIONS')}
          {columns.has('software') && th('SOFTWARE')}
          {columns.has('env') && th('ENV', 'env')}
          {columns.has('service') && th('SERVICE')}
          {columns.has('monitors') && th('MONITORS', 'monitors', 'ht-center')}
          {columns.has('cpu') && th('CPU USAGE %', 'cpu', 'ht-cpu')}
          {columns.has('errors') && th('ERRORS')}
        </tr>
      </thead>
      <tbody>
        {page.hosts.map((h) => {
          const header = h.group !== undefined && h.group !== lastGroup;
          lastGroup = h.group;
          return (
            <Fragment key={h.name}>
              {header && (
                <tr className="ht-group">
                  <td colSpan={colCount}>
                    {h.group} <span className="muted-text">({page.groups?.find((g) => g.key === h.group)?.count ?? 0} hosts)</span>
                  </td>
                </tr>
              )}
              <Row h={h} columns={columns} selected={h.name === selected} onOpen={onOpen} />
            </Fragment>
          );
        })}
      </tbody>
    </table>
  );
}
