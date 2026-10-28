import { UserRound } from 'lucide-react';
import { ProfileSetup } from './ProfileSetup';

/** On-Call → My On-Call Profile: how pages reach the signed-in member. */
export function ProfilePage() {
  return (
    <div className="onc-list-page onc-profile">
      <div className="onc-kicker">
        <UserRound size={14} /> My On-Call Profile
      </div>
      <h1 className="onc-profile-title">How pages reach you</h1>
      <p className="muted">ObserveCraft notifies responders by email and through your organization&apos;s Slack and webhook channels. Phone calls, SMS and push notifications aren&apos;t supported.</p>
      <section className="onc-card onc-setup">
        <ProfileSetup initialStep={3} />
      </section>
    </div>
  );
}
