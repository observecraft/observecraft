-- Database for the OTel Collector's ClickHouse exporter (traces + metrics tables are created by
-- the exporter itself with create_schema: true; it needs the database to exist first).
CREATE DATABASE IF NOT EXISTS otel;
