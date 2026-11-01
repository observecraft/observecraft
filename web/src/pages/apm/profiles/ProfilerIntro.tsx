import { Link } from 'react-router-dom';
import { ArrowRight, CircleDollarSign, Code2, Cpu, Gauge, Rocket } from 'lucide-react';
import { IntroPreview, ProductIntro, introButton } from '../../../components/ui/ProductIntro';

const BENEFITS = [
  { icon: Rocket, title: 'Fix production issues faster', text: 'See the code that was running when a request was slow, without reproducing it elsewhere.' },
  { icon: Gauge, title: 'Make services faster', text: 'CPU, wall time, locks and allocations per method and line, for a service, an endpoint or a single trace.' },
  { icon: CircleDollarSign, title: 'Spend less on compute', text: 'Find the methods that burn CPU and memory, and compare versions after a fix.' },
];

/** The profiler before any service sends profiles. */
export function ProfilerIntro() {
  return (
    <ProductIntro
      icon={Code2}
      product="Continuous Profiler"
      title="Find the code that slows your services down"
      text="Always-on profiles from the Datadog tracers show where production spends CPU, time and memory, down to the line."
      actions={
        <Link className={introButton.primary} to="/apm/instrument">
          Get Started <ArrowRight size={16} />
        </Link>
      }
      preview={
        <IntroPreview
          title="order-service · CPU time"
          rows={[
            ['OrderController.checkout', '31%'],
            ['PricingEngine.applyDiscounts', '18%'],
            ['InventoryClient.reserve', '12%'],
            ['JsonSerializer.write', '9%'],
            ['OrderRepository.save', '7%'],
          ]}
        />
      }
      benefits={BENEFITS}
      start={[
        {
          icon: <Cpu size={26} />,
          title: 'Turn on profiling in the tracer',
          text: 'Instrument the service with a Datadog tracer and set DD_PROFILING_ENABLED=true; the Agent forwards profiles here.',
          to: '/apm/instrument',
        },
      ]}
    />
  );
}
