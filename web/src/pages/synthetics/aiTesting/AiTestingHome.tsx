import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ArrowUp, Info, MapPin, MonitorSmartphone, Rocket, Sparkles, UserRound } from 'lucide-react';
import { DEVICES } from '../../../api/synthetics/browserSteps';
import { useSyntheticLocations } from '../queries';
import { useProfiles, useStartJob } from './queries';

const PROMPTS = ['Explore my website and generate tests', 'Check that visitors can fill out a contact form on my website without submitting it', 'Check that visitors can navigate my website'];

/** The first URL in a request (`https://…`, or a bare host such as shop.example.com). */
export function urlIn(text: string): string {
  const full = /https?:\/\/[^\s"'<>]+/i.exec(text)?.[0];
  if (full) return full.replace(/[.,;)]+$/, '');
  const bare = /\b((?:[a-z0-9-]+\.)+[a-z]{2,})(\/[^\s]*)?/i.exec(text);
  return bare ? `https://${bare[1]}${bare[2] ?? '/'}` : '';
}

/** New job: a request in words, then the job's parameters to confirm before anything runs. */
export function AiTestingHome() {
  const [request, setRequest] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const send = (text: string) => text.trim() && setRequest(text.trim());
  if (request === null) {
    return (
      <div className="ait-home">
        <div className="ait-hero">
          <span className="ait-mark">
            <Sparkles size={34} />
          </span>
          <div>
            <h1>Test with AI</h1>
            <p className="muted">Assist explores your site in a real browser and proposes browser tests for its key user journeys.</p>
          </div>
        </div>
        <Composer value={draft} onChange={setDraft} onSend={() => send(draft)} placeholder="Describe what you'd like to test, or enter your website URL…" />
        <h4 className="ait-try">Try a prompt</h4>
        <div className="ait-prompts">
          {PROMPTS.map((p) => (
            <button key={p} className="ait-prompt" onClick={() => send(p)}>
              <Sparkles size={15} /> {p}
            </button>
          ))}
        </div>
      </div>
    );
  }
  return <Conversation key={request} request={request} onRestart={() => setRequest(null)} />;
}

function Composer({ value, onChange, onSend, placeholder, disabled }: { value: string; onChange: (v: string) => void; onSend: () => void; placeholder: string; disabled?: boolean }) {
  return (
    <div className="ait-composer">
      <textarea
        rows={2}
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            onSend();
          }
        }}
        aria-label="What to test"
      />
      <button className="btn btn-primary ait-send" disabled={disabled || !value.trim()} onClick={onSend} title="Send">
        <ArrowUp size={16} />
      </button>
    </div>
  );
}

/** The request, the job card Assist prepared from it, and a box to refine it. */
function Conversation({ request, onRestart }: { request: string; onRestart: () => void }) {
  const navigate = useNavigate();
  const found = urlIn(request);
  // A prompt that is only a URL means: explore it.
  const prompt = found && request.replace(/https?:\/\/\S+/i, '').trim().length < 3 ? '' : PROMPTS[0] === request ? '' : request;
  const [startUrl, setStartUrl] = useState(found);
  const [restrict, setRestrict] = useState(true);
  const [deviceId, setDeviceId] = useState(DEVICES[0].id);
  const [profileId, setProfileId] = useState('');
  const [reply, setReply] = useState('');
  const [goal, setGoal] = useState(prompt);
  const profiles = useProfiles();
  const location = useSyntheticLocations().data?.[0];
  const start = useStartJob();
  const urlOk = /^https?:\/\/[^/\s]+/i.test(startUrl.trim());
  const run = () =>
    start.mutate({ startUrl: startUrl.trim(), restrictDomain: restrict, deviceId, prompt: goal, profileId: profileId || null }, { onSuccess: (r) => navigate(`/synthetics/ai-testing/jobs/${r.id}`) });
  return (
    <div className="ait-chat">
      <div className="ait-bubble user">{request}</div>
      <div className="ait-assistant">
        <span className="ait-assistant-name">
          <Sparkles size={15} /> Assist
        </span>
        <div className="ait-bubble assistant">
          <p>
            {goal ? 'I’ll test this in a browser' : 'I’ll explore your site in a browser'}
            {found ? ` starting from ${found}` : ''}. {found ? 'Check the job below, then start it.' : 'No URL was given: enter where to start below, then start the job.'}
          </p>
          <div className="ait-job">
            <h3>
              <Rocket size={16} /> {goal ? 'Test a journey' : 'Explore website'}
            </h3>
            <label className="form-label">
              Starting URL
              <input className="syn-input mono" value={startUrl} onChange={(e) => setStartUrl(e.target.value)} placeholder="Enter a URL to start from" autoFocus={!found} />
            </label>
            <p className="muted ait-hint">Assist performs real actions on the site (clicks, typing), so point it at a staging or test environment rather than production.</p>
            <label className="mon-check">
              <input type="checkbox" className="checkbox" checked={restrict} onChange={() => setRestrict((r) => !r)} />
              <span className="mon-check-label">Restrict exploration to the starting URL's domain</span>
            </label>
            <p className="muted ait-hint">Its subdomains included. Turn it off when signing in goes through another site.</p>
            <div className="ait-job-row">
              <label className="form-label">
                Location
                <span className="ait-select-icon">
                  <MapPin size={14} />
                  <select className="syn-select" value="observecraft-server" disabled aria-label="Location">
                    <option value="observecraft-server">{location?.name ?? 'ObserveCraft server'}</option>
                  </select>
                </span>
              </label>
              <label className="form-label">
                Device
                <span className="ait-select-icon">
                  <MonitorSmartphone size={14} />
                  <select className="syn-select" value={deviceId} onChange={(e) => setDeviceId(e.target.value)}>
                    {DEVICES.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.label} ({d.size})
                      </option>
                    ))}
                  </select>
                </span>
              </label>
              <label className="form-label">
                Agent profile
                <span className="ait-select-icon">
                  <UserRound size={14} />
                  <select className="syn-select" value={profileId} onChange={(e) => setProfileId(e.target.value)}>
                    <option value="">Anonymous visitor</option>
                    {(profiles.data ?? []).map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  </select>
                </span>
              </label>
            </div>
            {goal && (
              <label className="form-label">
                Goal
                <textarea className="syn-textarea" rows={2} value={goal} onChange={(e) => setGoal(e.target.value)} />
              </label>
            )}
            {location && !location.browser && <p className="syn-hint warn">AI Testing needs oc-browser, which this server doesn't use yet (set OC_BROWSER_URL and OC_BROWSER_TOKEN).</p>}
            {!profileId && (
              <div className="ait-info">
                <Info size={16} />
                <span>
                  Without an agent profile, Assist browses as an anonymous visitor: it can't sign in or fill in saved details. <Link to="/synthetics/ai-testing/profiles">Create a profile</Link> from
                  your global variables to let it sign in.
                </span>
              </div>
            )}
            {start.isError && <p className="form-error">{(start.error as Error).message}</p>}
            <div className="ait-row">
              <button className="btn btn-primary" disabled={!urlOk || start.isPending || location?.browser === false} onClick={run}>
                <Rocket size={14} /> {start.isPending ? 'Starting…' : profileId ? 'Start Exploration' : 'Start Exploration Without a Profile'}
              </button>
              <button className="btn" onClick={onRestart}>
                Start over
              </button>
            </div>
          </div>
          <p>Nothing has run yet. Once the job runs, I explore the site, propose tests for the journeys I find, and group them into suites for you to review.</p>
        </div>
      </div>
      <Composer
        value={reply}
        onChange={setReply}
        onSend={() => {
          const url = urlIn(reply);
          if (url) setStartUrl(url);
          const rest = reply.replace(/https?:\/\/\S+/i, '').trim();
          if (rest.length >= 3) setGoal(rest);
          setReply('');
        }}
        placeholder="Reply to Assist: a URL to start from, or what to test…"
      />
    </div>
  );
}
