import { useState } from 'react';
import { ArrowRight, BellRing, CircleDollarSign, FileUp, PiggyBank, Tags, Wallet } from 'lucide-react';
import { IntroPreview, ProductIntro, introButton } from '../../components/ui/ProductIntro';
import { SetupModal } from '../setup/SetupOptions';
import { cloudCostSetup } from './cloudCostSetup';

const BENEFITS = [
  { icon: Tags, title: 'Allocate every dollar', text: 'Break costs down by team, service, environment or any tag on the line items, and see who owns what.' },
  { icon: CircleDollarSign, title: 'Put costs next to telemetry', text: 'Cost metrics sit beside metrics, traces and logs in dashboards and notebooks, so engineers see what their changes cost.' },
  { icon: BellRing, title: 'Catch surprises early', text: 'Budgets track each month against a plan, and cost monitors alert when spend jumps.' },
];

/** Cloud Cost → Overview before any costs were uploaded. */
export function CloudCostIntro() {
  const [setup, setSetup] = useState(false);
  return (
    <>
      <ProductIntro
        icon={Wallet}
        product="Cloud Cost Management"
        title="Know what your cloud costs, and why"
        text="Bring in costs from cloud providers, SaaS vendors and AI APIs, allocate them to teams and services, and keep them under budget."
        actions={
          <button className={introButton.primary} onClick={() => setSetup(true)}>
            Add costs <ArrowRight size={16} />
          </button>
        }
        preview={
          <IntroPreview
            title="Spend this month"
            kpis={[
              ['Month to date', '$48,210'],
              ['Forecast', '$71,900'],
              ['vs last month', '+6%'],
            ]}
            bars={[42, 45, 44, 47, 51, 49, 53, 55, 52, 58, 61, 57, 60, 64]}
          />
        }
        benefits={BENEFITS}
        start={[
          { icon: <FileUp size={26} />, title: 'Upload a custom costs file', text: 'Send line items from any provider or vendor through the Cloud Cost API.', onClick: () => setSetup(true) },
          {
            icon: <PiggyBank size={26} />,
            title: 'Plan a budget',
            text: 'Set monthly amounts per team or service, and compare them with the actual costs once they arrive.',
            to: '/cloud-cost/budgets',
            cta: 'Open budgets',
          },
        ]}
      />
      {setup && <SetupModal title="Bring costs into ObserveCraft" options={cloudCostSetup(window.location.origin)} onClose={() => setSetup(false)} />}
    </>
  );
}
