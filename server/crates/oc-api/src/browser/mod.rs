//! Browser tests: steps in Datadog's browser step model, validated here and run by oc-browser (`browser/` at the
//! repo root, Playwright and Chromium in their own container, reached at `OC_BROWSER_URL`).
pub mod client;
pub mod steps;
