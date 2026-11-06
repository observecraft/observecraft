# Changelog

## [0.14.0](https://github.com/observecraft/observecraft/compare/v0.13.0...v0.14.0) (2026-11-06)


### Features

* **intake:** ingest is unlimited by default; instance admins can limit an organization ([b0c3348](https://github.com/observecraft/observecraft/commit/b0c33482c4ce6cb5e33da7b084161db8d349d12a))
* **logs:** log normalization in Rust, row for row as Vector's normalize.vrl ([a64306e](https://github.com/observecraft/observecraft/commit/a64306ee3d51921e142e4d11fbb13d756adc3713))
* **logs:** switching to the Kafka ingester starts where the earlier path stopped ([35abdd4](https://github.com/observecraft/observecraft/commit/35abdd45e706200a21521788b3f50cd157d5da6f))
* **logs:** the server reads the log topics from Kafka itself (OC_LOG_INGEST=kafka) ([bcfaa7d](https://github.com/observecraft/observecraft/commit/bcfaa7d1e4bc6c1c4cc192de4a9694d06e8b200e))
* **logs:** the server's Kafka log ingester is the default log path ([d9558f1](https://github.com/observecraft/observecraft/commit/d9558f150ff5fa91fa1ee1dc2436cf67001723b7))


### Bug fixes

* **logs:** the server starts while Kafka is unreachable ([37d38f5](https://github.com/observecraft/observecraft/commit/37d38f5df707723ba2f2b5717e8088abea3b26a2))
* **metrics:** keep ~3M active series flowing: series cache and collector memory defaults ([e70e975](https://github.com/observecraft/observecraft/commit/e70e975febfa9bbe16a6496d3767fa2de87eb2a7))


### Performance

* **intake:** an org's records spread over the topic's 12 partitions ([7050d87](https://github.com/observecraft/observecraft/commit/7050d87d77191e91c1cbf4ba59b7e0a2ddf7a9a8))
* **logs:** a partition's next batch is fetched while the last one is stored ([3c5fb5e](https://github.com/observecraft/observecraft/commit/3c5fb5e3915b6b92c41d75d3a56ecc25e31071b1))
* **logs:** larger, cheaper Kafka ingest batches ([f149b0c](https://github.com/observecraft/observecraft/commit/f149b0cc40dc8f4a68fccd4f45b47d48f30001bd))
* **logs:** lighter Kafka ingest path: live tail on demand, typed admission, unindexed-only raw copy ([9b3d40c](https://github.com/observecraft/observecraft/commit/9b3d40ce31972dff04db1fcef74b05b75ad7cc1c))
* **logs:** the Kafka ingester stores log attributes once, in attr ([a676d39](https://github.com/observecraft/observecraft/commit/a676d399d51a26c4f2c57f9d0e50804de633fc25))


### Documentation

* **benchmarks:** logs and metrics after the Kafka log ingester ([ae9c3dc](https://github.com/observecraft/observecraft/commit/ae9c3dc6c805c14be7014c41206779a87a08f3f8))

## [0.13.0](https://github.com/observecraft/observecraft/compare/v0.12.2...v0.13.0) (2026-11-04)


### Features

* **ci:** GitLab CI integration that pulls pipelines through GitLab's API ([c62ac68](https://github.com/observecraft/observecraft/commit/c62ac68a1881e6b6eb6636b2bf3f6f226807e21a))
* **cloud-cost:** recommendations for AWS and Azure resources ([a78d7c7](https://github.com/observecraft/observecraft/commit/a78d7c7fdf079ebb6dcc6c3b6f44f017bf1859ac))

## [0.12.2](https://github.com/observecraft/observecraft/compare/v0.12.1...v0.12.2) (2026-11-04)


### Bug fixes

* **helm:** bound the collector's memory on a Kafka backlog, optional KEDA scaling on consumer lag ([877cc30](https://github.com/observecraft/observecraft/commit/877cc30aaaf94b98e94954b6cc14b1e0bff144c8))

## [0.12.1](https://github.com/observecraft/observecraft/compare/v0.12.0...v0.12.1) (2026-11-03)


### Bug fixes

* **apm:** a service page shows its primary operation, inferred dependencies and statement shapes ([da7c974](https://github.com/observecraft/observecraft/commit/da7c974a1fee666a1dcb0f9cd34c285d4fd85bdb))
* **apm:** infer a dependency's host from the URL of its HTTP calls ([5120576](https://github.com/observecraft/observecraft/commit/51205766ea6ca844e656f3374df7dcad1ff3db43))
* **metrics:** a metric nobody sent has no data instead of a 400 ([1065cfb](https://github.com/observecraft/observecraft/commit/1065cfb192c7d5897e2082318a318d9c78fe393a))
* read ClickHouse rows without the client's schema validation ([bc6e935](https://github.com/observecraft/observecraft/commit/bc6e935cc4289ed9ee5b09eec2b2eef96664c9ab))
* **web:** search boxes kept in the URL no longer lose characters ([e0a88ef](https://github.com/observecraft/observecraft/commit/e0a88ef66ae39d15f2977532d3bb503ed76a9bc4))

## [0.12.0](https://github.com/observecraft/observecraft/compare/v0.11.0...v0.12.0) (2026-11-03)


### Features

* **assist:** a run's progress as a readable timeline ([#80](https://github.com/observecraft/observecraft/issues/80)) ([9df7518](https://github.com/observecraft/observecraft/commit/9df7518b296345c6543b73cba23550c129530aea))


### Bug fixes

* **cloud-cost:** read an Azure export file again once it changed ([#81](https://github.com/observecraft/observecraft/issues/81)) ([0591501](https://github.com/observecraft/observecraft/commit/05915017a1f400ea1deee24daf4be35b7ee95118))

## [0.11.0](https://github.com/observecraft/observecraft/compare/v0.10.1...v0.11.0) (2026-11-03)


### Features

* **assist:** Assist Chat, the Ask Assist panel, settings and usage reports ([c3dbddc](https://github.com/observecraft/observecraft/commit/c3dbddca8ddf5e1b021e90abf386f6480b910d48))
* **assist:** chat with the org's data through tools, with approval for changes ([f498b3c](https://github.com/observecraft/observecraft/commit/f498b3c9f97cb77b568dcdc9eff182677b6f40fa))
* **assist:** each organization sets its own model provider ([109d6a9](https://github.com/observecraft/observecraft/commit/109d6a9c07afb4e5742c8440ec0e5b563fa265cd))
* **assist:** Investigation, Security Analyst, Agent Builder, Code, AI Credits and Usage pages ([42199fe](https://github.com/observecraft/observecraft/commit/42199fe0660a97b41cf842bea909858f2c39a1a6))
* **assist:** investigations, security verdicts, agents, credits and code sessions ([83c2a88](https://github.com/observecraft/observecraft/commit/83c2a8816678c95b378be0f1a066992a2d5a03e7))
* **cloud-cost:** read AWS and Azure bills ([#78](https://github.com/observecraft/observecraft/issues/78)) ([6fa3ff6](https://github.com/observecraft/observecraft/commit/6fa3ff6cd983a3c57a2892afda8451fbfbb91809))
* **errors:** investigate an Error Tracking issue and propose its fix with Assist ([093d6bb](https://github.com/observecraft/observecraft/commit/093d6bbf6151ae856a3604c76c698efb1382e91a))
* **incidents:** incident pages closer to the reference, investigated by Assist ([4ee1504](https://github.com/observecraft/observecraft/commit/4ee1504112e0620e041326fda2ae8411f6d12ec3))


### Bug fixes

* **assist:** Assist Chat's Reports and Settings side by side in the menu ([b83889d](https://github.com/observecraft/observecraft/commit/b83889d3cf29bc39756675d35a72ac250b3c822e))

## [0.10.1](https://github.com/observecraft/observecraft/compare/v0.10.0...v0.10.1) (2026-11-02)


### Bug fixes

* **azure:** skip the types and batches Azure Monitor refuses instead of failing the subscription ([0b26a1a](https://github.com/observecraft/observecraft/commit/0b26a1a18414f1df8fa692501c89b20ac8e1f324))

## [0.10.0](https://github.com/observecraft/observecraft/compare/v0.9.0...v0.10.0) (2026-11-02)


### Features

* **charts:** server.podLabels, for keyless cloud identities such as AKS Workload Identity ([a4c5b7f](https://github.com/observecraft/observecraft/commit/a4c5b7fa07a3a924c342f20d0661f16ed9a99734))


### Dependencies

* axum 0.8 and utoipa 6 ([4a5a628](https://github.com/observecraft/observecraft/commit/4a5a6283d0205d8d16d705b11969b6f7e27a6712))
* rand 0.10, aes-gcm 0.11, argon2 0.6, hmac 0.13, sha1/sha2 0.11 ([53fcac1](https://github.com/observecraft/observecraft/commit/53fcac11d43ad6cdeca5dc1255556cd259092fd6))
* sqlx 0.9 ([1114941](https://github.com/observecraft/observecraft/commit/1114941f18bdf720001781052db7b77e7b62d3c0))
* thiserror 2, base64 0.23, zstd 0.14, x509-cert 0.3, rusty-s3 0.11; CI on Rust 1.99 ([9a50961](https://github.com/observecraft/observecraft/commit/9a50961f0bae05e1003356202ab1293aca3bc2a1))
* tonic and prost 0.14, clickhouse 0.15, tower-http 0.7 ([5beeec7](https://github.com/observecraft/observecraft/commit/5beeec7aafe4830dd1ba050e33036737509af036))

## [0.9.0](https://github.com/observecraft/observecraft/compare/v0.8.0...v0.9.0) (2026-11-02)


### Features

* **dashboards:** AWS, Azure and Google Cloud overview and per-service dashboards ([c75c831](https://github.com/observecraft/observecraft/commit/c75c831910de0dcc830c8bc5f881c1a7055eb5cb))
* **dashboards:** every integrations-core dashboard, installed when its integration reports ([f2430a4](https://github.com/observecraft/observecraft/commit/f2430a4f0466a4b079309c9b395c0420fb45ebb5))
* **dashboards:** sunburst widgets as donuts with rings per group, total and legend ([daf1f36](https://github.com/observecraft/observecraft/commit/daf1f36aff41c77748ce2657c8ae70017f8418ca))
* **errors:** track errors as Datadog's Error Tracking does ([4d2a0ad](https://github.com/observecraft/observecraft/commit/4d2a0adf6c6e9747114247f16eeec096a8070436))
* **integrations:** AWS, Azure and Google Cloud integrations with their metric crawlers ([d8e3eaf](https://github.com/observecraft/observecraft/commit/d8e3eaf0a0a59b84691151cb638db2fbf7f2911d))
* **integrations:** Community tab with integrations-extras in place of the marketplace ([f85ea8f](https://github.com/observecraft/observecraft/commit/f85ea8f6a5b099a340d2b2caad68ddeeb1c7b75b))
* **integrations:** integrations-core catalog, original logos and integration details ([d343bad](https://github.com/observecraft/observecraft/commit/d343bad7795ebca8743394090510e19e49bc8d10))
* **integrations:** integrations-extras community checks and log integrations, Traefik shown as Traefik ([cd64ed3](https://github.com/observecraft/observecraft/commit/cd64ed333bacaa17cc60e37272e90da683b279fa))
* **kubernetes:** Kubernetes integration, autoscaling, remediation and live rollouts ([a46c54e](https://github.com/observecraft/observecraft/commit/a46c54eba57df4f317719708a97ea0e04647b549))
* **logs:** grok rubyhash filter ([f4e559b](https://github.com/observecraft/observecraft/commit/f4e559b4c55f5d171d68952c11a09caefb513b66))
* **network:** add shared grouping icons and service map logos ([#70](https://github.com/observecraft/observecraft/issues/70)) ([06a849e](https://github.com/observecraft/observecraft/commit/06a849e00f640b4115c3b879f316e1e1a60899b3))
* **on-call:** empty states, policy dialog and page review analytics ([b1312d2](https://github.com/observecraft/observecraft/commit/b1312d2d4076e552d009b783a26a063cc6b66288))
* **web:** product intro pages for products without data ([6319cb0](https://github.com/observecraft/observecraft/commit/6319cb0ddbb83d2fb6ac7647c58b76cd9fa2fecb))


### Bug fixes

* **apm:** keep long resources inside the attention cards ([bff4981](https://github.com/observecraft/observecraft/commit/bff4981e0cfe61dac9e6d68d86a860e57276a093))
* **cloud-cost:** escape service names in the waterfall tooltip ([7879b29](https://github.com/observecraft/observecraft/commit/7879b29647667007f932c9538ef7c0c263911000))
* **integrations:** CodeQL findings in SVG sanitizing, README comments, entity decoding and the AWS forwarder link ([373cba8](https://github.com/observecraft/observecraft/commit/373cba80cea9d2626a3dc0681e39a595f952948c))
* **integrations:** each SVG sanitizer removal repeats on its own until stable ([c2f6fe1](https://github.com/observecraft/observecraft/commit/c2f6fe1ac3709a61c366b009594c246a21b30aff))
* **integrations:** SVG sanitizer loops written inline, one replacement per statement ([79a0021](https://github.com/observecraft/observecraft/commit/79a002135449bd0337ab3fc78d6661d8118c4a29))
* **logs:** integration pipelines from every integrations-core integration compile ([6dae575](https://github.com/observecraft/observecraft/commit/6dae575d0d69eb42eaafac6634bdad04488e9ca5))
* **logs:** read the level of lines that pad it ([8a087df](https://github.com/observecraft/observecraft/commit/8a087dfa4bc3664e278af36055568194e67d1518))


### Performance

* **dashboards:** reduce one-value widgets on the server and send series compactly ([ac1833d](https://github.com/observecraft/observecraft/commit/ac1833d08ee673873d366336baa7eda820daf506))


### Documentation

* add maintainer sponsorship links ([a733322](https://github.com/observecraft/observecraft/commit/a73332269ecadd2bb698513776b3f2517516e300))
* integrations-extras notes ([1d6af82](https://github.com/observecraft/observecraft/commit/1d6af8229107efd960dceeecb1db0a4da3e06a98))
* integrations, cloud and Kubernetes notes for agents and the README ([9fdf939](https://github.com/observecraft/observecraft/commit/9fdf9397ed34c6dcc306d37642321c4a3a9e4d20))
* **integrations:** guide to installing and building integrations, linked from Community ([7707001](https://github.com/observecraft/observecraft/commit/7707001d086aafb362b015742bae322847849774))
* refresh README screenshots with current UI ([5e8cfd3](https://github.com/observecraft/observecraft/commit/5e8cfd3a27f1a497596183383b0c1d07d740d83d))


### Dependencies

* bump oras-project/setup-oras ([29f8edf](https://github.com/observecraft/observecraft/commit/29f8edf480381fc39d14fb235635d6f693bb4a6f))
* bump rust in /server in the docker group ([c710ca3](https://github.com/observecraft/observecraft/commit/c710ca3e7ec46b134c02aa4c529470953f2cc48c))
* bump the npm group across 1 directory with 6 updates ([e80b527](https://github.com/observecraft/observecraft/commit/e80b5273657527032b15fc261ac4365ce7a63f21))
* React 19, react-grid-layout 2, simple-icons 16 and the other npm major updates (TypeScript stays on 5) ([43bfa17](https://github.com/observecraft/observecraft/commit/43bfa17b81d9b565ad45c71371801ac0098c2c10))
* regenerate logos and cloud artwork for @iconify-json/logos 1.2.18 ([9549a9d](https://github.com/observecraft/observecraft/commit/9549a9d67c9420f97e937b5652db444a09de3eed))

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
