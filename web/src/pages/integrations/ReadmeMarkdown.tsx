// An integrations-core README section (BSD-3-Clause, shown as it is): GitHub-flavoured Markdown rendered as
// React elements (raw HTML is not rendered), code blocks with a copy button, Host / Docker / Kubernetes tabs.
import { useState, type ReactNode } from 'react';
import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { CodeBlock } from '../../components/code/CodeBlock';
import { readmeSegments } from './readme';

/** Text of a code element's children (react-markdown passes the code as string children). */
const text = (children: ReactNode): string => (Array.isArray(children) ? children.map(text).join('') : typeof children === 'string' ? children : '');

const COMPONENTS: Components = {
  a: ({ href, children }) =>
    href && /^https?:\/\//.test(href) ? (
      <a href={href} target="_blank" rel="noreferrer noopener">
        {children}
      </a>
    ) : (
      <>{children}</>
    ),
  img: ({ src, alt }) => (typeof src === 'string' && /^https:\/\//.test(src) ? <img src={src} alt={alt ?? ''} loading="lazy" referrerPolicy="no-referrer" /> : null),
  // Fenced blocks are a <pre><code>; the CodeBlock draws its own frame.
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }) => {
    const code = text(children).replace(/\n$/, '');
    return className?.startsWith('language-') || code.includes('\n') ? <CodeBlock code={code} /> : <code>{children}</code>;
  },
  table: ({ children }) => (
    <div className="readme-table">
      <table className="im-table">{children}</table>
    </div>
  ),
};

function Part({ markdown }: { markdown: string }) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS}>
      {markdown}
    </ReactMarkdown>
  );
}

function Tabs({ tabs }: { tabs: { name: string; markdown: string }[] }) {
  const [active, setActive] = useState(0);
  return (
    <div className="readme-tabs">
      <div className="readme-tab-row" role="tablist">
        {tabs.map((t, i) => (
          <button key={t.name} role="tab" aria-selected={i === active} className={i === active ? 'active' : ''} onClick={() => setActive(i)}>
            {t.name}
          </button>
        ))}
      </div>
      <div className="readme-tab-body">
        <Part markdown={tabs[active]?.markdown ?? ''} />
      </div>
    </div>
  );
}

export function ReadmeMarkdown({ source }: { source: string }) {
  return <div className="readme">{readmeSegments(source).map((s, i) => (s.kind === 'tabs' ? <Tabs key={i} tabs={s.tabs} /> : <Part key={i} markdown={s.markdown} />))}</div>;
}
