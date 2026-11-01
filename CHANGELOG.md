# Changelog

## [0.8.0](https://github.com/observecraft/observecraft/compare/v0.7.1...v0.8.0) (2026-11-01)


### Features

* **ingestion:** preserve JSON log identities across Kafka replay ([1efeb61](https://github.com/observecraft/observecraft/commit/1efeb61b53532410f09fdf872e68cba2f36abdaf))
* **logs:** deduplicate replayed storage and aggregate event identities ([f66016a](https://github.com/observecraft/observecraft/commit/f66016af2055d3280ee2442b9026ecf952fa7975))


### Bug fixes

* **apm:** preserve span aggregate metrics after ingestion rebase ([97c4a8d](https://github.com/observecraft/observecraft/commit/97c4a8d98c91ff4c6df54d6f442cd3b54a854db6))
* **benchmarks:** pin proxy peers to configured upstreams ([a6ea6d6](https://github.com/observecraft/observecraft/commit/a6ea6d6c558d4b2031c5766c179116e3249e86a9))
* **ingestion:** align replay checks with rebased migrations ([b8256ee](https://github.com/observecraft/observecraft/commit/b8256eecbefedc5f7cea27cd85f352e7ee107676))
* **ingestion:** preserve OTLP log identities across Kafka replay ([055f06d](https://github.com/observecraft/observecraft/commit/055f06d21dc0f32c82a5082de8b031fb2829271c))
* **ingestion:** retain Kafka ownership for new RUM exporters ([4206e93](https://github.com/observecraft/observecraft/commit/4206e93bdba928a5609c83d0fce9945441839dee))
* **ingestion:** retain Kafka ownership through export and repair late rollups ([e0bc973](https://github.com/observecraft/observecraft/commit/e0bc97392b2dd6ff66baf04a4f6758eb473273ca))
* **ingestion:** scope storage pools to each server runtime ([a460bfe](https://github.com/observecraft/observecraft/commit/a460bfe4b3ae07b06683e924b34e00308dcfb576))
* **logs:** bound aggregate memory and preserve raw retention during backfill ([1924d52](https://github.com/observecraft/observecraft/commit/1924d52a1e76a9c5c40d7180509a791cf7514e60))
* **logs:** coordinate durable index quotas across API replicas ([fbcb3e9](https://github.com/observecraft/observecraft/commit/fbcb3e952de8220ffed6812cf1c44517bf68d10d))
* **logs:** deduplicate generated metric contributions through replay ([b3c50ea](https://github.com/observecraft/observecraft/commit/b3c50eac00e83fdfcd9a438f2a6d074069d9e577))
* **logs:** persist generated metric exports before acknowledging ingestion ([042586a](https://github.com/observecraft/observecraft/commit/042586a0f30f4599b3b956310099157162c406d5))
* **logs:** preserve quota admissions across failed writes and replay ([6a5f7c9](https://github.com/observecraft/observecraft/commit/6a5f7c9a316ddc693a5731507b82543696b4e4ee))
* **tests:** handle billing month boundaries and JSON responses ([7f677da](https://github.com/observecraft/observecraft/commit/7f677da04552c2480972457d52d4baae99776ff8))


### Performance

* **ingestion:** add isolated throughput and backpressure benchmarks ([4daf1ee](https://github.com/observecraft/observecraft/commit/4daf1ee6919e7e39c7a69ca6df39ea7164d38f34))
* **ingestion:** expose bounded stage timing histograms ([29d1f09](https://github.com/observecraft/observecraft/commit/29d1f09be26ab079c0e0f4d5d9f173a6b52942d0))
* **logs:** coalesce indexed writes and reuse storage connections ([dcac9ca](https://github.com/observecraft/observecraft/commit/dcac9cad4ca59d6586a436d02c0b0f838ed44d6a))
* **logs:** release quota gates before bounded storage IO ([df43252](https://github.com/observecraft/observecraft/commit/df43252311ceda2d985fbb1079fd0f3fc0b66ae8))
* **logs:** skip unused Vector resource and identity work ([ec3142e](https://github.com/observecraft/observecraft/commit/ec3142ea5333f89a3260542ecc60532babc745e9))
* **metrics:** coalesce bounded outbox exports across spool entries ([e0e977b](https://github.com/observecraft/observecraft/commit/e0e977b304d8c7ba32f82b601fa736d3ad7b96b1))


### Documentation

* **contributing:** preserve commits with linear rebase merges ([95904f6](https://github.com/observecraft/observecraft/commit/95904f6fb0c3165d637b30d4462ac48110c8ceca))
* **ingestion:** record batching and sustained-load validation ([58dec1d](https://github.com/observecraft/observecraft/commit/58dec1d416361a239ad896ed012a5e36a9e82132))

## [0.7.1](https://github.com/observecraft/observecraft/compare/v0.7.0...v0.7.1) (2026-10-31)


### Bug fixes

* **helm:** keep Kafka data out of the volume root ([#55](https://github.com/observecraft/observecraft/issues/55)) ([59f07b1](https://github.com/observecraft/observecraft/commit/59f07b11b04a09fc9a8d940e6a44f82eae37320a))

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
