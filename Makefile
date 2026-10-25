# ObserveCraft monorepo tasks.
COMPOSE := docker compose -f deploy/docker-compose.yml

.PHONY: up down logs ps server intake tracegen web web-mock test openapi build clean-data

up:            ## Start storage, pipeline and synthetic load (logs are indexed by `make server` on the host)
	OC_LOGPROC_URL=http://host.docker.internal:8090/ingest/logs $(COMPOSE) up -d
down:          ## Stop the stack (keeps data)
	$(COMPOSE) down
clean-data:    ## Stop the stack and delete ClickHouse/Postgres volumes
	$(COMPOSE) down -v
logs:          ## Tail stack logs
	$(COMPOSE) logs -f --tail=100
ps:
	$(COMPOSE) ps

# Local-only ingest key for the demo generators (the server creates it for the default org).
DEMO_KEY ?= ocik_local_demo_only_do_not_use_in_production

server:        ## Run the API on :8080 against the local stack
	cd server && OC_DEMO_INGEST_KEY=$(DEMO_KEY) OC_DEMO_ARCHIVE_ENDPOINT=http://localhost:9100 cargo run --bin observecraft-server
intake:        ## Run the telemetry intake (OTLP/HTTP + JSON logs, API-key auth) on :4318
	cd server && cargo run --bin oc-intake
tracegen:      ## Send synthetic traces, host metrics and CI runs through the intake
	cd server && OC_INGEST_KEY=$(DEMO_KEY) cargo run --bin oc-tracegen
web:           ## Run the web app on :5173 against the API
	cd web && VITE_API_MODE=live npm run dev
web-mock:      ## Run the web app with in-browser mocks (no backend needed)
	cd web && VITE_API_MODE=mock npm run dev

test:          ## Rust tests + web typecheck
	cd server && cargo test
	cd web && npx tsc -p .

openapi:       ## Regenerate web/src/api/generated/* from the Rust API definition
	cd server && cargo run -q --bin observecraft-server -- openapi > ../web/src/api/generated/openapi.json
	cd web && npx openapi-typescript src/api/generated/openapi.json -o src/api/generated/schema.d.ts

build:         ## Production builds
	cd server && cargo build --release
	cd web && npm run build
