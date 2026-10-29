# ObserveCraft monorepo tasks.
COMPOSE := docker compose -f deploy/docker-compose.yml

.PHONY: up down cluster-up cluster-down logs ps server intake tracegen load-orgs load storage-report web web-mock test openapi build images charts clean-data

up:            ## Start storage, cache, pipeline and synthetic load (logs are indexed by `make server` on the host)
	OC_LOGPROC_URL=http://host.docker.internal:8090/ingest/logs $(COMPOSE) up -d
down:          ## Stop the stack (keeps data)
	$(COMPOSE) down
# 2-shard x 2-replica ClickHouse cluster `oc` with Keeper on :18161-18164 (deploy/compose/clickhouse-cluster.yml).
CLUSTER_COMPOSE := docker compose -f deploy/compose/clickhouse-cluster.yml
cluster-up:    ## Start the ClickHouse cluster (CLUSTER_PROFILE=pipeline adds a collector on :18169)
	$(CLUSTER_COMPOSE) $(if $(CLUSTER_PROFILE),--profile $(CLUSTER_PROFILE)) up -d --wait
cluster-down:  ## Stop the ClickHouse cluster and delete its data
	$(CLUSTER_COMPOSE) --profile pipeline down -v
clean-data:    ## Stop the stack and delete ClickHouse/Postgres volumes
	$(COMPOSE) down -v
logs:          ## Tail stack logs
	$(COMPOSE) logs -f --tail=100
ps:
	$(COMPOSE) ps

# Local-only ingest key for the demo generators (the server creates it for the default org).
DEMO_KEY ?= ocik_local_demo_only_do_not_use_in_production

server:        ## Run the API on :8080 against the local stack
	cd server && OC_CACHE_URL=$${OC_CACHE_URL:-redis://localhost:6379/0} OC_DEMO_INGEST_KEY=$(DEMO_KEY) OC_DEMO_ARCHIVE_ENDPOINT=http://localhost:9100 cargo run --bin observecraft-server
intake:        ## Run the telemetry intake (OTLP HTTP :4318 / gRPC :4317, Datadog APIs; API-key auth)
	cd server && cargo run --bin oc-intake
tracegen:      ## Send synthetic traces, host metrics and CI runs through the intake
	cd server && OC_INGEST_KEY=$(DEMO_KEY) cargo run --bin oc-tracegen
# Storage load profile and report (docs/adr/0002): `make load-orgs`, `make load`, then `make storage-report`.
LOAD_ORGS ?= 4
LOAD_POSTGRES_URL ?= postgres://observecraft:observecraft@localhost:5432/observecraft
load-orgs:     ## Create load-org-1..N and their ingest keys (local only; after the server's migrations)
	psql "$(LOAD_POSTGRES_URL)" -v orgs=$(LOAD_ORGS) -f scripts/load-orgs.sql
load:          ## Send the multi-org metrics load profile through the intake (OC_LOAD_* settings)
	cd server && OC_LOAD_ORGS=$(LOAD_ORGS) cargo run --release --bin oc-tracegen -- load
storage-report: ## Rows/bytes written, merges, CPU by query kind, top SELECT shapes (WINDOW=1h, CLUSTER=name)
	node scripts/storage-report.mjs --window $(or $(WINDOW),1h) $(if $(CLUSTER),--cluster $(CLUSTER))
web:           ## Run the web app on :5173 against the API
	cd web && VITE_API_MODE=live npm run dev
web-mock:      ## Run the web app with in-browser mocks (no backend needed)
	cd web && VITE_API_MODE=mock npm run dev

test:          ## Rust tests, web typecheck, integration dashboard tests
	cd server && cargo test
	cd web && npx tsc -p .
	node --test scripts/*.test.mjs

openapi:       ## Regenerate web/src/api/generated/* from the Rust API definition
	cd server && cargo run -q --bin observecraft-server -- openapi > ../web/src/api/generated/openapi.json
	cd web && npx openapi-typescript src/api/generated/openapi.json -o src/api/generated/schema.d.ts

build:         ## Production builds
	cd server && cargo build --release
	cd web && npm run build
images:        ## Container image: observecraft (server, intake, web app)
	docker build -f server/Dockerfile -t observecraft:dev .
charts:        ## Lint and render the Helm charts
	helm lint deploy/helm/observecraft
	helm lint deploy/helm/observecraft -f deploy/helm/observecraft/values-ha.yaml
	helm lint deploy/helm/observecraft-agent -f deploy/helm/observecraft-agent/ci/default-values.yaml
	helm template oc deploy/helm/observecraft > /dev/null
	helm template oc deploy/helm/observecraft-agent -f deploy/helm/observecraft-agent/ci/default-values.yaml > /dev/null
