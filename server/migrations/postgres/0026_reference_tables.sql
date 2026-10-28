-- Reference tables (Integrations → Reference Tables): CSV files uploaded by users, one row per
-- primary key value, to look values up by key (owners of a service, names of a product SKU, …).
-- Uploading a new file replaces every row of the table. See routes/reference_tables.rs.

CREATE TABLE reference_tables (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- Lowercase letters, digits and underscores (`service_owners`); it can't be changed.
    name        TEXT NOT NULL CHECK (name ~ '^[a-z0-9_]{1,100}$'),
    description TEXT NOT NULL DEFAULT '',
    -- The file's header, in file order: ["sku", "product_name", "category"].
    columns     JSONB NOT NULL,
    -- One of `columns`; its values are unique and never empty.
    primary_key TEXT NOT NULL,
    -- Name of the uploaded file, as the browser reported it.
    file_name   TEXT NOT NULL DEFAULT '',
    row_count   INTEGER NOT NULL DEFAULT 0,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name),
    -- Target of the rows' (org, table) key.
    UNIQUE (org_id, id)
);

CREATE TABLE reference_table_rows (
    org_id     TEXT NOT NULL,
    table_id   UUID NOT NULL,
    -- The row's primary key value.
    key        TEXT NOT NULL,
    -- Position in the uploaded file (0-based), the order rows are listed in.
    position   INTEGER NOT NULL,
    -- Every value of the row by column name: {"sku": "SKU-0001", "category": "kitchen"}.
    data       JSONB NOT NULL,
    PRIMARY KEY (table_id, key),
    FOREIGN KEY (org_id, table_id) REFERENCES reference_tables(org_id, id) ON DELETE CASCADE
);

CREATE INDEX reference_table_rows_position ON reference_table_rows (org_id, table_id, position);
