# Implementation plan: browser tests and AI Testing (AI test generation)

Synthetic Monitoring has API tests (HTTP, SSL, DNS, TCP) only. Datadog's Bits Testing explores a site from a starting
URL, maps its user journeys and generates browser tests (plus API and network tests) grouped into suites to review.
Browser tests come first; the agent then explores with the same browser and proposes tests in that model. Ours is
called AI Testing: Bits is Datadog's brand, which ObserveCraft doesn't use.

Decisions:
- `oc-browser` (TypeScript + Playwright, Apache-2.0) is its own container: it runs steps (screenshot, URL, error
  and duration per step, a Playwright trace on failure) and serves agent sessions (accessibility snapshot with
  refs, actions on refs that come back with a stable locator, network log, saved `storageState`). Stateless: no
  database, no tenancy; the server calls it over internal HTTP with a shared token (`OC_BROWSER_URL`,
  `OC_BROWSER_TOKEN`).
- The Rust server keeps the platform: the step model and its validation (`browser/steps.rs`, the API contract),
  scheduling, monitors, results, screenshots, metrics, credentials, TOTP, and the agent (Assist's loop).
- SSRF: every request the browser makes is resolved and refused when internal (same rules as `synthetics_net`,
  `OC_SYNTHETICS_ALLOW_PRIVATE`), before and after DNS; the container has no other egress.
- Steps follow Datadog's browser step model (step types, `params.element.userLocator`, `allowFailure`,
  `isCritical`, `timeout`).
- Login and SSO are a real browser following redirects; the agent signs in with an agent profile's credentials
  (secure variables, TOTP); sessions are reused with `storageState`. No SSO-specific code.
- Later (not in this plan): pentesting on the same `oc-browser` API (snapshot, act, network log, replay), with
  Strix (Apache-2.0) or Nuclei (MIT) plus our agent; never ZAP; domain ownership verified first.

## Stage 1: oc-browser runs steps
Goal: the server validates steps (`browser/steps.rs`); `oc-browser` (`browser/` at the repo root) runs them in a
fresh context (go to URL, click, type, select, hover, press key, scroll, refresh, wait, assertions on
page/element/URL/JavaScript, extract variable), retrying assertions until their timeout, refusing internal
addresses, with a screenshot, URL, duration, error and failure code per step.
Success Criteria: Rust validation tests; oc-browser tests against local pages in a real Chromium (each step type,
allowFailure/isCritical, variables, timeouts, blocked private addresses, the token).
Status: Complete (Rust: 4 validation tests; oc-browser: 25 tests, 8 in a real Chromium; per-run proxy per
context; the Playwright trace on failure moved to Stage 2 with result storage)

## Stage 2: Browser tests in Synthetic Monitoring
Goal: `browser` test type with steps, scheduled and run like API tests (retries, monitor state, metrics
`synthetics.browser.*`, results with step details; screenshots kept 30 days); Chromium in Compose and Helm;
New Test → Browser Test editor, test page and result detail with step screenshots; mocks.
Success Criteria: route tests (create, validate, run, record); `make test`; the editor and a run seen in the
browser, live and mock.
Status: Complete (video of failed and manual runs; screenshots and videos in ClickHouse; the Helm chart's
`browser.enabled` stays off until the release workflow publishes the oc-browser image; no Playwright trace yet)

## Stage 3: AI Testing agent
Goal: a `testing` run explores from a starting URL (optionally restricted to its domain, optionally as an agent
profile with saved credentials) with browser tools (open, read the page's interactive elements, click, type, back),
compares journeys with existing tests and proposes suites (browser tests, plus HTTP/SSL tests of the hosts it
met); also "generate tests from a description". Proposals are stored for review; accepting creates the tests and
the suite.
Success Criteria: tools and proposal validation tested with a scripted model and browser; accept/reject routes
tested; tenancy on every query.
Status: Complete (oc-browser sessions with refs and computed locators; `testing` runs, 80 turns; proposals made of
recorded actions and sample-run before they are stored; accept → tests every 15 minutes + a suite per journey; agent
profiles name global variables. Not yet: approvals before actions ("ask about every action"), API and network test
proposals, proposals shown while the job runs (they are saved when it ends), a run with a real model)

## Stage 4: AI Testing UI
Goal: Synthetics → AI Testing tab: prompt with suggestions, conversation with the job card (starting URL,
restrict to domain, location, agent profile), the run's live timeline, Suites to review (edit, accept, reject),
all jobs; agent profiles in Synthetics settings; mocks.
Success Criteria: typecheck, mock and live run seen in the browser; docs and CLAUDE.md updated.
Status: Complete (Synthetics → AI Testing: prompt and job card, the job's live timeline (Assist's RunView) and its
proposed suites, Suites to review, All jobs, Agent profiles; mocks; seen in mock mode, routes checked live. Not yet: a
job with a real model)
