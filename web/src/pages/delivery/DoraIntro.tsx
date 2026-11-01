import { Link } from 'react-router-dom';
import { ArrowRight, BarChart3, GitCommitHorizontal, LifeBuoy, Rocket, ShieldAlert } from 'lucide-react';
import { IntroPreview, ProductIntro, introButton } from '../../components/ui/ProductIntro';

const BENEFITS = [
  { icon: Rocket, title: 'Deployment frequency', text: 'How often each service reaches production, from the deploy jobs of your CI pipelines.' },
  { icon: GitCommitHorizontal, title: 'Lead time for changes', text: 'How long a commit takes to run in production, from the first commit of a deployment to its finish.' },
  { icon: ShieldAlert, title: 'Change failure rate', text: 'The share of deployments that fail, or that an incident follows within a day.' },
  { icon: LifeBuoy, title: 'Time to restore service', text: 'How long incidents on these services take to resolve, from Incident Response.' },
];

/** DORA Metrics before any CI deploy job reported an environment. */
export function DoraIntro() {
  return (
    <ProductIntro
      icon={BarChart3}
      product="DORA Metrics"
      title="Measure how fast, and how safely, you ship"
      text="The four DORA metrics per service and environment, computed from your CI pipelines' deploy jobs, their commits and the incidents that follow."
      actions={
        <Link className={introButton.primary} to="/delivery/settings#add-repository">
          Get Started <ArrowRight size={16} />
        </Link>
      }
      preview={
        <IntroPreview
          title="Production, past 7 days"
          kpis={[
            ['Deploys / day', '6.4'],
            ['Lead time', '5h 20m'],
            ['Failure rate', '4%'],
          ]}
          bars={[5, 7, 6, 8, 4, 2, 3, 7, 9, 6, 8, 7, 3, 2]}
        />
      }
      benefits={BENEFITS}
      start={[
        {
          icon: <Rocket size={26} />,
          title: 'Report deploy jobs from CI',
          text: 'Send your pipelines to CI Visibility and give deploy jobs the deployment.environment.name attribute.',
          to: '/delivery/settings#add-repository',
        },
        { icon: <LifeBuoy size={26} />, title: 'Declare incidents', text: 'Incidents on a service count toward change failure rate and time to restore.', to: '/incidents', cta: 'Open incidents' },
      ]}
    />
  );
}
