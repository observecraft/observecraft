import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { BellRing, FileCode2, Gauge, LayoutTemplate, Plus, Radar, Target } from 'lucide-react';
import { IntroPreview, ProductIntro, introButton } from '../../components/ui/ProductIntro';

const BENEFITS = [
  {
    icon: Radar,
    title: 'Check endpoints around the clock',
    text: 'HTTP, SSL, DNS and TCP tests run on a schedule from the ObserveCraft server, with assertions on status, body, headers and timings.',
  },
  { icon: BellRing, title: 'Alert before users notice', text: 'A failing test alerts through your notification channels and can page the team that owns the endpoint.' },
  { icon: Gauge, title: 'Track uptime and latency', text: 'Every run writes synthetics.* metrics for dashboards and SLOs.' },
  { icon: Target, title: 'Close coverage gaps', text: 'APM endpoints that receive traffic but have no test are listed below, ready to turn into tests.' },
];

/** Synthetics → Overview before the first test. `children`: what still makes sense without tests (coverage gaps). */
export function SyntheticsIntro({ children }: { children?: ReactNode }) {
  return (
    <ProductIntro
      icon={Radar}
      product="Synthetic Monitoring"
      title="Test your endpoints before your users do"
      text="Scheduled API tests check that your services answer, quickly and correctly, and alert the right team when they don't."
      actions={
        <>
          <Link className={introButton.primary} to="/synthetics/create">
            <Plus size={16} /> New Test
          </Link>
          <Link className={introButton.secondary} to="/synthetics/templates">
            Start from a template
          </Link>
        </>
      }
      preview={
        <IntroPreview
          title="API tests · past hour"
          kpis={[
            ['Passing', '24 / 25'],
            ['Uptime', '99.96%'],
            ['p95', '182 ms'],
          ]}
          rows={[
            ['GET storefront-api /health', 'OK · 41 ms'],
            ['POST order-service /orders', 'OK · 168 ms'],
            ['SSL api.example.com', 'OK · 61 days left'],
            ['GET billing /invoices', 'Alert · 503'],
          ]}
        />
      }
      benefits={BENEFITS}
      start={[
        {
          icon: <FileCode2 size={26} />,
          title: 'Create an API test',
          text: 'Pick a request type, the URL and what the answer must look like, then a schedule and who to notify.',
          to: '/synthetics/create',
          cta: 'New test',
        },
        {
          icon: <LayoutTemplate size={26} />,
          title: 'Start from a template',
          text: 'Ready-made tests for health checks, certificates, DNS records and open ports.',
          to: '/synthetics/templates',
          cta: 'Browse templates',
        },
      ]}
    >
      {children}
    </ProductIntro>
  );
}
