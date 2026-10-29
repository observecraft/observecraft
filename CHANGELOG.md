# Changelog

## [0.3.1](https://github.com/observecraft/observecraft/compare/v0.3.0...v0.3.1) (2026-10-29)


### Bug fixes

* **ci:** include the workflow in the release image's build provenance ([#37](https://github.com/observecraft/observecraft/issues/37)) ([61883bd](https://github.com/observecraft/observecraft/commit/61883bde2e995f2b01dba15cce6bd2bcbaf094c8))

## [0.3.0](https://github.com/observecraft/observecraft/compare/v0.2.2...v0.3.0) (2026-10-28)


### Features

* **integrations:** faithful Kubernetes dashboards with units, notes and banners ([#33](https://github.com/observecraft/observecraft/issues/33)) ([77ee9c5](https://github.com/observecraft/observecraft/commit/77ee9c59ad33d4de0b58aba9358982f346be922b))


### Bug fixes

* **sso:** accept OIDC issuers that end with a slash ([#32](https://github.com/observecraft/observecraft/issues/32)) ([fa71aa4](https://github.com/observecraft/observecraft/commit/fa71aa4d1d6ed0fdb13d2ab9f7250b466498688b))

## [0.2.2](https://github.com/observecraft/observecraft/compare/v0.2.1...v0.2.2) (2026-10-27)


### ⚠ BREAKING CHANGES

* **metrics:** collectors write to the new metric storage. Existing metric history in the old exporter tables is not migrated and is no longer queried. The old tables remain until their TTL expires. Review the metrics storage ADR before upgrading.

### Features

* **integrations:** add live Kubernetes dashboards and shared queries ([#29](https://github.com/observecraft/observecraft/issues/29)) ([52fb11e](https://github.com/observecraft/observecraft/commit/52fb11ed2048445e164b6f5ad65c39d88c2e4b25))
* **metrics:** series, samples and rollups storage ([#28](https://github.com/observecraft/observecraft/issues/28)) ([ed49350](https://github.com/observecraft/observecraft/commit/ed49350476768b84de261897be51a09e06ca068f))


### Bug fixes

* **metrics:** require a present tag value for wildcard tag filters, preventing unrelated workloads from inflating dashboard totals.
* **collector:** store attribute maps in a stable order ([#26](https://github.com/observecraft/observecraft/issues/26)) ([bd59299](https://github.com/observecraft/observecraft/commit/bd592996928a26b3fdffbeac81d8cd1e1b9e2432))


### Dependencies

* Bump @remix-run/router and react-router-dom in /web ([#21](https://github.com/observecraft/observecraft/issues/21)) ([530c3cb](https://github.com/observecraft/observecraft/commit/530c3cb38076093902bace3f16bcaa0a90e10266))
* Bump vite from 5.4.11 to 8.3.5 in /web ([#22](https://github.com/observecraft/observecraft/issues/22)) ([45e3738](https://github.com/observecraft/observecraft/commit/45e3738cafcefd1122d569e5542f0b30060119a2))
* upgrade React Router to 7.18.4 ([#30](https://github.com/observecraft/observecraft/issues/30)) ([8d3c8d6](https://github.com/observecraft/observecraft/commit/8d3c8d60956f485071f9e8b1fa0aa4d4b87978ff))

## [0.2.1](https://github.com/observecraft/observecraft/compare/v0.2.0...v0.2.1) (2026-10-26)


### Bug fixes

* **clickhouse:** keep a small bundled ClickHouse from restarting under load ([#25](https://github.com/observecraft/observecraft/issues/25)) ([7612916](https://github.com/observecraft/observecraft/commit/7612916a5112df86ac239d835de908dd0468307e))
* **sso:** send a User-Agent to the OIDC identity provider ([#23](https://github.com/observecraft/observecraft/issues/23)) ([847b576](https://github.com/observecraft/observecraft/commit/847b5765dc082e652e8becb0e75b2ce1be4450f1))

## [0.2.0](https://github.com/observecraft/observecraft/compare/v0.1.0...v0.2.0) (2026-10-26)


### Features

* **helm:** Artifact Hub listing for both charts ([#19](https://github.com/observecraft/observecraft/issues/19)) ([fa27c7e](https://github.com/observecraft/observecraft/commit/fa27c7ee56e141125ffeab3d8b9a0fd1eb969db7))

## 0.1.0 (2026-10-26)


### Features

* Datadog-compatible intake, Helm charts and new product pages ([#2](https://github.com/observecraft/observecraft/issues/2)) ([6f7511a](https://github.com/observecraft/observecraft/commit/6f7511ae38ebfa230ed3a0d38f2438e9f87c7827))
