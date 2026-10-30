import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from 'react-router-dom';
import { CircleDollarSign, Code2, Plus } from 'lucide-react';
import { listBudgets } from '../../api/cloudCost/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { money } from './costModel';
import { ShareBar } from './widgets';

export const monthLabel = (m: number) => new Date(Date.UTC(Math.floor(m / 100), (m % 100) - 1, 1)).toLocaleDateString('en-US', { month: 'short', year: 'numeric', timeZone: 'UTC' });

const EXAMPLE = `resource "datadog_cost_budget" "ml_platform" {
  name          = "ML platform"
  metrics_query = "all.cost{*} by {team}"
  start_month   = 202601
  end_month     = 202602

  budget_line {
    amounts = { "202601" = 60000, "202602" = 60000 }
    tag_filters {
      tag_key   = "team"
      tag_value = "ml"
    }
  }
}`;

/** Budgets are kept as Datadog's budget API keeps them, so its Terraform resource manages the same ones. */
function TerraformNote() {
  return (
    <div className="ccm-budget-help">
      <p>
        Budgets are the records of the Cloud Cost budget API (<code>PUT /api/v2/cost/budget</code>): Datadog's Terraform provider (<code>datadog_cost_budget</code>,{' '}
        <code>datadog_cost_custom_forecast</code>) and API clients manage the same budgets with an application key.
      </p>
      <pre className="ccm-code">{EXAMPLE}</pre>
    </div>
  );
}

/** Plan → Budgets: the org's budgets with this month's cost and forecast, or how to start one. */
export function BudgetsPage() {
  const navigate = useNavigate();
  const [showTerraform, setShowTerraform] = useState(false);
  const q = useQuery({ queryKey: ['cloud-cost', 'budgets'], queryFn: ({ signal }) => listBudgets(signal) });
  if (q.isPending) return <Loading label="Loading budgets…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!q.data.length)
    return (
      <div className="ccm-page">
        <div className="ccm-overview">
          <div className="ccm-hero">
            <span className="ccm-hero-product">
              <CircleDollarSign size={22} /> Cloud Cost Management
            </span>
            <h1>Budget and track your cloud cost spend</h1>
            <p>Define a budget for your cloud costs. Once you've created a budget, your organization will be able to visualize and track it here.</p>
            <div className="ccm-hero-actions">
              <Link className="btn btn-primary" to="/cloud-cost/budgets/new">
                <Plus size={14} /> Create a New Budget
              </Link>
              <button className="btn" onClick={() => setShowTerraform((s) => !s)}>
                <Code2 size={14} /> Manage with Terraform
              </button>
            </div>
            {showTerraform && <TerraformNote />}
          </div>
        </div>
      </div>
    );
  return (
    <div className="ccm-page">
      <div className="ccm-overview">
        <table className="ccm-table ccm-budget-list">
          <thead>
            <tr>
              <th>NAME</th>
              <th>MONTHS</th>
              <th className="num">THIS MONTH'S BUDGET</th>
              <th>SPENT THIS MONTH</th>
              <th className="num">FORECAST</th>
              <th className="num">TOTAL BUDGET</th>
            </tr>
          </thead>
          <tbody>
            {q.data.map((b) => {
              const over = b.month != null && b.monthForecast > b.monthAmount;
              return (
                <tr key={b.id} className="ccm-clickable" onClick={() => navigate(`/cloud-cost/budgets/${b.id}`)}>
                  <td>
                    <Link to={`/cloud-cost/budgets/${b.id}`} onClick={(e) => e.stopPropagation()}>
                      <b>{b.name}</b>
                    </Link>
                    <div className="ccm-mono muted-text">{b.metricsQuery}</div>
                  </td>
                  <td>
                    {monthLabel(b.startMonth)} – {monthLabel(b.endMonth)}
                  </td>
                  <td className="num">{b.month != null ? money(b.monthAmount) : '—'}</td>
                  <td>
                    {b.month != null ? (
                      <div className="ccm-coverage">
                        <ShareBar share={b.monthAmount ? b.monthActual / b.monthAmount : 0} tone={over ? 'red' : 'blue'} />
                        <span>{money(b.monthActual)}</span>
                      </div>
                    ) : (
                      '—'
                    )}
                  </td>
                  <td className={`num${over ? ' ccm-up' : ''}`}>{b.month != null ? money(b.monthForecast) : '—'}</td>
                  <td className="num">{money(b.totalAmount)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <button className="link-btn ccm-terraform-toggle" onClick={() => setShowTerraform((s) => !s)}>
          <Code2 size={14} /> Manage budgets with Terraform
        </button>
        {showTerraform && <TerraformNote />}
      </div>
    </div>
  );
}
