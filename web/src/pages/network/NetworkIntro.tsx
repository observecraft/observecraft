import { useState } from 'react';
import { ArrowRight, Globe, Network, RefreshCcw, Share2, Waypoints } from 'lucide-react';
import { IntroPreview, ProductIntro, introButton } from '../../components/ui/ProductIntro';
import { SetupModal } from '../setup/SetupOptions';
import { NETWORK_SETUP } from './networkSetup';

const BENEFITS = [
  { icon: Share2, title: 'Map every dependency', text: 'Traffic between services, pods, hosts and availability zones, including the ones nobody documented.' },
  { icon: RefreshCcw, title: 'Spot unhealthy links', text: 'Volume, TCP retransmits, round-trip time and resets per connection show where the network hurts.' },
  { icon: Globe, title: 'Follow DNS', text: 'Which servers answer which lookups, how fast, and how many fail or time out.' },
  { icon: Waypoints, title: 'Keep cross-zone traffic in check', text: 'Group by availability zone or region to find the chatty links that cost money.' },
];

/** Cloud Network Monitoring before the Agent's system-probe sends connections. */
export function NetworkIntro() {
  const [setup, setSetup] = useState(false);
  return (
    <>
      <ProductIntro
        icon={Network}
        product="Cloud Network Monitoring"
        title="See the traffic between everything you run"
        text="Connections between Kubernetes pods, containers, hosts, services and availability zones, with their volume, retransmits and latency, collected by the Datadog Agent."
        actions={
          <button className={introButton.primary} onClick={() => setSetup(true)}>
            Get Started <ArrowRight size={16} />
          </button>
        }
        preview={
          <IntroPreview
            title="Traffic by service"
            kpis={[
              ['Volume sent', '2.4 GB'],
              ['Retransmits', '0.02%'],
              ['RTT', '1.3 ms'],
            ]}
            rows={[
              ['storefront-api → order-service', '812 MB'],
              ['order-service → billing', '455 MB'],
              ['api-gateway → catalog-service', '390 MB'],
              ['inventory-service → postgres', '214 MB'],
            ]}
          />
        }
        benefits={BENEFITS}
      />
      {setup && <SetupModal title="Send network connections to ObserveCraft" options={NETWORK_SETUP} onClose={() => setSetup(false)} />}
    </>
  );
}
