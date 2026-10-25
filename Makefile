# ObserveCraft monorepo tasks.
COMPOSE := docker compose -f deploy/docker-compose.yml

.PHONY: up down logs ps server tracegen web web-mock test openapi build clean-data

up:            ## Start storage, pipeline and synthetic load
	$(COMPOSE) up -d
down:          ## Stop the stack (keeps data)
	$(COMPOSE) down
clean-data:    ## Stop the stack and delete ClickHouse/Postgres volumes
	$(COMPOSE) down -v
logs:          ## Tail stack logs
	$(COMPOSE) logs -f --tail=100
ps:
	$(COMPOSE) ps

server:        ## Run the API on :8080 against the local stack
	cd server && cargo run --bin observecraft-server
tracegen:      ## Send synthetic OTLP traces (APM demo data) to the local collector
	cd server && cargo run --bin oc-tracegen
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
