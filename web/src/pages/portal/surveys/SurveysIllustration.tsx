import { Briefcase, Globe, Mail, Workflow } from 'lucide-react';
import { BrandLogo } from '../../../components/brand/BrandLogo';
import { LIKERT_LABELS } from '../../../api/portalSurveys/logic';

const QUESTIONS: [string, number][] = [
  ['I can get a service running locally without help', 3],
  ['CI gives me feedback fast enough to stay focused', 4],
];

/** Weekly form views, for the small chart card. */
const VIEWS = [12, 18, 15, 26, 22, 31, 28, 36, 30, 22];

/** First-run picture: a survey form, its views, and what can happen when someone submits it. */
export function SurveysIllustration() {
  const max = Math.max(...VIEWS);
  const points = VIEWS.map((v, i) => `${(i * 200) / (VIEWS.length - 1)},${54 - (v / max) * 46}`).join(' ');
  return (
    <div className="psv-art" aria-hidden="true">
      <div className="psv-art-form">
        <div className="psv-art-form-bar" />
        <div className="psv-art-form-body">
          <div className="psv-art-form-title">Developer Experience Survey</div>
          <div className="psv-art-form-sub">Tell us how building and shipping feels this quarter.</div>
          {QUESTIONS.map(([q, picked], qi) => (
            <div key={q} className="psv-art-question">
              <div className="psv-art-question-title">
                {qi + 1}. {q}
              </div>
              <div className="psv-art-likert">
                {LIKERT_LABELS.map((label, i) => (
                  <div key={label} className="psv-art-likert-option">
                    <span className={`psv-art-radio${i === picked ? ' on' : ''}`} />
                    <span className="psv-art-likert-label">{label}</span>
                  </div>
                ))}
              </div>
            </div>
          ))}
          <div className="psv-art-submit">Submit</div>
        </div>
      </div>

      <div className="psv-art-views">
        <div className="psv-art-views-head">
          <span>Form views</span>
          <strong>240</strong>
        </div>
        <svg viewBox="0 0 200 60" className="psv-art-views-chart">
          <polyline points={`0,60 ${points} 200,60`} className="psv-art-views-area" />
          <polyline points={points} className="psv-art-views-line" />
        </svg>
      </div>

      <div className="psv-art-submitted">
        <div className="psv-art-submitted-title">Choose what happens on form submission</div>
        <div className="psv-art-tiles">
          <div className="psv-art-tile">
            <BrandLogo name="slack" size={18} />
            <span>Slack message</span>
          </div>
          <div className="psv-art-tile">
            <Mail size={18} className="psv-art-tile-icon mail" />
            <span>Send email</span>
          </div>
          <div className="psv-art-tile">
            <Briefcase size={18} className="psv-art-tile-icon case" />
            <span>Create case</span>
          </div>
          <div className="psv-art-tile">
            <Globe size={18} className="psv-art-tile-icon http" />
            <span>HTTP request</span>
          </div>
          <div className="psv-art-tile">
            <Workflow size={18} className="psv-art-tile-icon flow" />
            <span>Run workflow</span>
          </div>
        </div>
      </div>
    </div>
  );
}
