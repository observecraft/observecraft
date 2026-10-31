# Changelog

## [0.7.0](https://github.com/observecraft/observecraft/compare/v0.6.0...v0.7.0) (2026-10-31)


### Features

* Synthetics, RUM, Session Replay, Cloud SIEM, Cloud Security and App & API Protection ([#52](https://github.com/observecraft/observecraft/issues/52)) ([f795222](https://github.com/observecraft/observecraft/commit/f79522275ecaa9d558812311638613c9b6afc560))
* Workload Protection, AI Guard, APM Settings and Logs Configuration ([#54](https://github.com/observecraft/observecraft/issues/54)) ([e494ff4](https://github.com/observecraft/observecraft/commit/e494ff4c429b329df0bc1b6c21a49da41761562c))


### Dependencies

* bump echarts from 5.5.1 to 6.1.0 in /web ([#31](https://github.com/observecraft/observecraft/issues/31)) ([6b3c232](https://github.com/observecraft/observecraft/commit/6b3c23296a7662352e83d866df4839d53158e46e))

## [0.6.0](https://github.com/observecraft/observecraft/compare/v0.5.1...v0.6.0) (2026-10-31)


### ⚠ BREAKING CHANGES

* Cloud Cost, Code Security, Kafka ingestion buffer, APM and portal pages ([#49](https://github.com/observecraft/observecraft/issues/49))

### Features

* Cloud Cost, Code Security, Kafka ingestion buffer, APM and portal pages ([#49](https://github.com/observecraft/observecraft/issues/49)) ([657e583](https://github.com/observecraft/observecraft/commit/657e583efea3fb85dc991bc2752eb6a3e5c93694))


### Bug fixes

* **security:** code scanning findings and vulnerable dependencies ([#51](https://github.com/observecraft/observecraft/issues/51)) ([0c9e1bd](https://github.com/observecraft/observecraft/commit/0c9e1bdc8f8ae4dbe19b8b384c9b13758e54c19a))

## [0.5.1](https://github.com/observecraft/observecraft/compare/v0.5.0...v0.5.1) (2026-10-30)


### Bug fixes

* **ci:** stabilize release checks and reduce pipeline build costs ([#47](https://github.com/observecraft/observecraft/issues/47)) ([7991aff](https://github.com/observecraft/observecraft/commit/7991affe14592ce3f5ae8ba44979c1fd14b9092e))

## [0.5.0](https://github.com/observecraft/observecraft/compare/v0.4.0...v0.5.0) (2026-10-29)


### ⚠ BREAKING CHANGES

* dashboard widget cache across live loads, explicit Valkey password with existing secrets ([#45](https://github.com/observecraft/observecraft/issues/45))

### Features

* **api:** Datadog-compatible management API for Terraform, Pulumi and SDKs ([#44](https://github.com/observecraft/observecraft/issues/44)) ([0c363ee](https://github.com/observecraft/observecraft/commit/0c363ee58719536b02857d0c518c8998d3d95f5a))


### Bug fixes

* dashboard widget cache across live loads, explicit Valkey password with existing secrets ([#45](https://github.com/observecraft/observecraft/issues/45)) ([f3741d4](https://github.com/observecraft/observecraft/commit/f3741d4b8045c82bdcfe202ddf9ef3062ae7a5e5))

## [0.4.0](https://github.com/observecraft/observecraft/compare/v0.3.2...v0.4.0) (2026-10-29)


### ⚠ BREAKING CHANGES

* **storage:** telemetry storage at SaaS scale ([#42](https://github.com/observecraft/observecraft/issues/42))

### Performance

* **storage:** telemetry storage at SaaS scale ([#42](https://github.com/observecraft/observecraft/issues/42)) ([f6450b8](https://github.com/observecraft/observecraft/commit/f6450b839021afc6a5e10ca778ac5d0111cf6bab))

## [0.3.2](https://github.com/observecraft/observecraft/compare/v0.3.1...v0.3.2) (2026-10-29)


### Bug fixes

* **ci:** build the release provenance like attest-build-provenance and tolerate re-signing ([#40](https://github.com/observecraft/observecraft/issues/40)) ([8c8376c](https://github.com/observecraft/observecraft/commit/8c8376cb1004e177ad60a2f85d31b4d060b1dac5))

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
