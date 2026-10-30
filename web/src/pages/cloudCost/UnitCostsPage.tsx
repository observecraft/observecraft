import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useSearchParams } from 'react-router-dom';
import { ArrowDown, ArrowUp, ChevronsLeft, ChevronsRight, PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import { unitCosts, type UnitCostRow } from '../../api/cloudCost/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { DAY, RANGES, filterQuery, money, rangeLabel, today, useCostTags, type Filters } from './costModel';
import { CostFilterBar, ProviderIcon, ShareBar } from './widgets';

interface UnitTable {
  title: string;
  scope: string;
  columns: string[];
  /** Price per hour (instances) or per pricing unit (everything else). */
  per: 'hour' | 'unit';
  note: { title: string; text: string[] };
}

interface UnitBoard {
  provider: string;
  label: string;
  title: string;
  tables: [UnitTable, UnitTable];
}

const ONDEMAND_NOTE = (provider: string, programs: string) => `${provider} on-demand costs already include negotiated discounts. The effective discount is what ${programs} took off on top of them.`;

const BOARDS: UnitBoard[] = [
  {
    provider: 'aws',
    label: 'AWS',
    title: 'AWS EC2 Pricing and Unit Costs',
    tables: [
      {
        title: 'EC2 Instance Hourly Pricing',
        scope: 'providername:aws servicename:AmazonEC2 pricingunit:Hrs',
        columns: ['aws_instance_type', 'regionname', 'aws_usage_type', 'aws_operation'],
        per: 'hour',
        note: {
          title: 'EC2 Instances',
          text: [
            'The **average hourly cost** and **discount rate** of each instance type, from your amortized spend. Only instance types you ran in the time frame are listed.',
            ONDEMAND_NOTE('AWS', 'Savings Plans and Reserved Instances'),
          ],
        },
      },
      {
        title: 'AWS Unit Pricing (excluding EC2 Instances)',
        scope: 'providername:aws -servicename:AmazonEC2',
        columns: ['servicename', 'aws_usage_type', 'regionname', 'pricingunit'],
        per: 'unit',
        note: {
          title: 'Everything else',
          text: [
            'The **average cost per unit** and **discount rate** of everything but EC2 instances. Only usage in the time frame is listed.',
            ONDEMAND_NOTE('AWS', 'Savings Plans and Reserved Instances'),
          ],
        },
      },
    ],
  },
  {
    provider: 'azure',
    label: 'Azure',
    title: 'Azure VM Pricing and Unit Costs',
    tables: [
      {
        title: 'Azure Virtual Machine Hourly Pricing',
        scope: 'providername:azure metercategory:"Virtual Machines"',
        columns: ['metername', 'regionname', 'additionalinfo_vcpus', 'additionalinfo_servicetype'],
        per: 'hour',
        note: {
          title: 'Virtual Machines',
          text: [
            'The **average hourly cost** and **discount rate** of your VMs, from your amortized spend. Only VMs you ran in the time frame are listed.',
            ONDEMAND_NOTE('Azure', 'reservations and savings plans'),
          ],
        },
      },
      {
        title: 'Azure Unit Pricing (excluding Virtual Machines)',
        scope: 'providername:azure -metercategory:"Virtual Machines"',
        columns: ['productname', 'metername', 'regionname', 'pricingunit', 'consumedservice'],
        per: 'unit',
        note: {
          title: 'Everything else',
          text: ['The **average cost per unit** and **discount rate** of everything but VMs. Only usage in the time frame is listed.', ONDEMAND_NOTE('Azure', 'reservations and savings plans')],
        },
      },
    ],
  },
  {
    provider: 'gcp',
    label: 'GCP',
    title: 'GCP Compute Engine Pricing and Unit Costs',
    tables: [
      {
        title: 'Compute Engine Hourly Pricing',
        scope: 'providername:gcp servicename:"Compute Engine" pricingunit:hour',
        columns: ['gcp_sku_description', 'gcp_machine_type', 'regionname'],
        per: 'hour',
        note: {
          title: 'Compute Engine',
          text: [
            'The **average hourly cost** and **discount rate** of your Compute Engine SKUs, from your amortized spend. Only SKUs used in the time frame are listed.',
            ONDEMAND_NOTE('Google Cloud', 'committed use discounts'),
          ],
        },
      },
      {
        title: 'GCP Unit Pricing (excluding Compute Engine)',
        scope: 'providername:gcp -servicename:"Compute Engine"',
        columns: ['servicename', 'gcp_sku_description', 'regionname', 'pricingunit'],
        per: 'unit',
        note: {
          title: 'Everything else',
          text: [
            'The **average cost per unit** and **discount rate** of everything but Compute Engine. Only usage in the time frame is listed.',
            ONDEMAND_NOTE('Google Cloud', 'committed use discounts'),
          ],
        },
      },
    ],
  },
];

const VISIBLE_FILTERS = 5;

const unitPrice = (v: number) => `$${v < 0.0001 && v > 0 ? v.toExponential(2) : v.toFixed(4)}`;

/** `**bold**` spans of a note. */
function Rich({ text }: { text: string }) {
  return <p>{text.split(/(\*\*[^*]+\*\*)/).map((part, i) => (part.startsWith('**') ? <b key={i}>{part.slice(2, -2)}</b> : part))}</p>;
}

type SortKey = string | 'amortized' | 'ondemand' | 'discount' | 'total';

function PriceTable({ table, filters, range }: { table: UnitTable; filters: Filters; range: { from: number; to: number } }) {
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'total', desc: true });
  const query = filterQuery(filters, table.scope);
  const q = useQuery({
    queryKey: ['cloud-cost', 'unit-costs', query, table.columns, range.from, range.to],
    queryFn: ({ signal }) => unitCosts({ query, range, groupBy: table.columns }, signal),
    placeholderData: (prev) => prev,
  });
  const hours = (r: UnitCostRow) => r.usageQuantity || 1;
  const rows = useMemo(() => {
    const all = (q.data ?? []).map((r) => ({
      r,
      amortized: r.amortizedCost / hours(r),
      ondemand: r.onDemandCost / hours(r),
      discount: r.onDemandCost > 0 ? Math.max(0, 1 - r.amortizedCost / r.onDemandCost) : 0,
      total: r.amortizedCost,
    }));
    const s = search.toLowerCase();
    const shown = s ? all.filter((x) => Object.values(x.r.tags).some((v) => v.toLowerCase().includes(s))) : all;
    const val = (x: (typeof all)[number]) => (sort.key in x && sort.key !== 'r' ? (x[sort.key as 'total'] as number) : (x.r.tags[sort.key] ?? ''));
    return shown.sort((a, b) => {
      const [va, vb] = [val(a), val(b)];
      const c = typeof va === 'number' && typeof vb === 'number' ? va - vb : String(va).localeCompare(String(vb));
      return sort.desc ? -c : c;
    });
  }, [q.data, search, sort]);
  const max = Math.max(1, ...rows.map((x) => x.total));
  const per = table.per === 'hour' ? 'PER HOUR' : 'PER UNIT';
  const head = (key: SortKey, label: string, num = false) => (
    <th key={key} className={num ? 'num' : undefined}>
      <button className="ccm-sort" onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : num }))}>
        {sort.key === key && (sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)} {label}
      </button>
    </th>
  );
  return (
    <div className="ccm-uc-row">
      <div className="ccm-uc-note">
        <h3>{table.note.title}</h3>
        {table.note.text.map((t) => (
          <Rich key={t} text={t} />
        ))}
      </div>
      <div className="ccm-widget ccm-uc-table">
        <div className="ccm-widget-head">
          <span>{table.title}</span>
        </div>
        <div className="ccm-widget-body">
          <label className="ccm-search ccm-uc-search">
            <Search size={15} />
            <input placeholder="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
          </label>
          {q.isPending ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="ccm-uc-scroll">
              <table className="ccm-table">
                <thead>
                  <tr>
                    {table.columns.map((c) => head(c, c.toUpperCase()))}
                    {head('amortized', `AMORTIZED ${per}`, true)}
                    {head('ondemand', `ON-DEMAND ${per}`, true)}
                    {head('discount', 'EFFECTIVE DISCOUNT', true)}
                    {head('total', 'TOTAL SPEND', true)}
                  </tr>
                </thead>
                <tbody>
                  {rows.length ? (
                    rows.map((x) => (
                      <tr key={JSON.stringify(x.r.tags)}>
                        {table.columns.map((c) => (
                          <td key={c} title={x.r.tags[c]}>
                            {x.r.tags[c]}
                          </td>
                        ))}
                        <td className="num">{unitPrice(x.amortized)}</td>
                        <td className="num">{unitPrice(x.ondemand)}</td>
                        <td className="num">{(x.discount * 100).toFixed(x.discount > 0 ? 2 : 0)} %</td>
                        <td className="num ccm-uc-total">
                          <span>{money(x.total)}</span>
                          <ShareBar share={x.total / max} />
                        </td>
                      </tr>
                    ))
                  ) : (
                    <tr>
                      {table.columns.map((c) => (
                        <td key={c}>N/A</td>
                      ))}
                      <td className="num">—</td>
                      <td className="num">—</td>
                      <td className="num">0.00 %</td>
                      <td className="num">—</td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Plan → Unit Costs: per provider, what each instance type and usage unit costs, amortized and on demand. */
export function UnitCostsPage() {
  const [params, setParams] = useSearchParams();
  const [collapsed, setCollapsed] = useState(false);
  const board = BOARDS.find((b) => b.provider === params.get('provider')) ?? BOARDS[0];
  const [filters, setFilters] = useState<Filters>({});
  const [allFilters, setAllFilters] = useState(false);
  const [rangeKey, setRangeKey] = useState('1mo');
  const [shift, setShift] = useState(0);
  const days = (RANGES.find((r) => r.value === rangeKey) ?? RANGES[2]).days;
  const range = useMemo(() => {
    const end = today() - shift * days * DAY;
    return { from: end - (days - 1) * DAY, to: end + DAY - 1 };
  }, [days, shift]);
  const tags = useCostTags(range, `providername:${board.provider}`);
  const keys = [...new Set(board.tables.flatMap((t) => t.columns))];
  const shown = allFilters ? keys : keys.slice(0, VISIBLE_FILTERS);
  const pick = (p: string) => (setParams({ provider: p }, { replace: true }), setFilters({}));
  return (
    <div className={`ccm-boards${collapsed ? ' collapsed' : ''}`}>
      <aside className="ccm-boards-nav">
        <button className="ccm-collapse" onClick={() => setCollapsed((c) => !c)}>
          {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />} {!collapsed && 'Collapse'}
        </button>
        {!collapsed && (
          <>
            <h4>Unit Costs Dashboards</h4>
            {BOARDS.map((b) => (
              <button key={b.provider} className={`ccm-board-link${b === board ? ' active' : ''}`} onClick={() => pick(b.provider)}>
                <ProviderIcon provider={b.provider} /> {b.label}
              </button>
            ))}
          </>
        )}
      </aside>
      <div className="ccm-boards-main">
        <div className="ccm-board">
          <div className="ccm-board-head">
            <h2>
              <ProviderIcon provider={board.provider} size={22} /> {board.title}
            </h2>
          </div>
          <div className="ccm-board-vars ccm-uc-vars">
            <CostFilterBar prefix="" keys={shown} filters={filters} onChange={setFilters} tags={tags.data}>
              {keys.length > VISIBLE_FILTERS && (
                <button className="link-btn" onClick={() => setAllFilters((a) => !a)}>
                  {allFilters ? 'Show less' : `+${keys.length - VISIBLE_FILTERS} more`}
                </button>
              )}
            </CostFilterBar>
            <span className="ccm-range">
              <Select
                label="UTC"
                value={rangeKey}
                options={RANGES.map((r) => ({ value: r.value, label: shift ? rangeLabel(range.from, range.to) : r.label, icon: <span className="ccm-period">{r.value}</span> }))}
                onChange={(v) => (setRangeKey(v), setShift(0))}
                align="right"
                triggerClassName="dd-trigger ccm-period-select"
              />
              <span className="btn-group">
                <button className="btn btn-sm" title="Earlier" onClick={() => setShift((s) => s + 1)}>
                  <ChevronsLeft size={14} />
                </button>
                <button className="btn btn-sm" title="Later" disabled={shift === 0} onClick={() => setShift((s) => Math.max(0, s - 1))}>
                  <ChevronsRight size={14} />
                </button>
              </span>
            </span>
          </div>
          {board.tables.map((t) => (
            <PriceTable key={`${board.provider}:${t.title}`} table={t} filters={filters} range={range} />
          ))}
        </div>
      </div>
    </div>
  );
}
