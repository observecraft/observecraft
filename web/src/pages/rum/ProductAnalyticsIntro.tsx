import { Link } from 'react-router-dom';
import { ArrowRight, ChartLine, Filter, Flame, PlayCircle, Users } from 'lucide-react';
import { IntroPreview, ProductIntro, introButton } from '../../components/ui/ProductIntro';

const BENEFITS = [
  { icon: Users, title: 'Know who comes back', text: 'Active users, retention and session length per application, country and device.' },
  { icon: Filter, title: 'Find where people drop off', text: 'Funnels over views and actions show which step of a journey loses users.' },
  { icon: Flame, title: 'See what gets clicked', text: 'Heatmaps and replays of real sessions show how a page is actually used.' },
  { icon: PlayCircle, title: 'Connect usage to performance', text: 'The same sessions carry load times and errors, so a slow page and its lost users are one click apart.' },
];

/** Product Analytics before any RUM application exists: it reads the sessions RUM collects. */
export function ProductAnalyticsIntro() {
  return (
    <ProductIntro
      icon={ChartLine}
      product="Product Analytics"
      title="Understand how people use your product"
      text="See which features get used, where users get stuck and what brings them back, from the sessions your RUM applications already collect."
      actions={
        <>
          <Link className={introButton.primary} to="/rum/applications?new=1">
            Set Up <ArrowRight size={16} />
          </Link>
          <Link className={introButton.secondary} to="/rum">
            About RUM
          </Link>
        </>
      }
      preview={
        <IntroPreview
          title="Product pulse"
          kpis={[
            ['Active users', '18.4k'],
            ['Session duration', '6m 12s'],
            ['Bounce rate', '23%'],
          ]}
          bars={[31, 34, 29, 36, 40, 38, 44, 47, 43, 49, 52, 50, 55, 58]}
        />
      }
      benefits={BENEFITS}
      start={[
        {
          icon: <Users size={26} />,
          title: 'Create a RUM application',
          text: 'Add the browser or mobile SDK to your app; its sessions feed Product Analytics.',
          to: '/rum/applications?new=1',
          cta: 'Create application',
        },
      ]}
    />
  );
}
