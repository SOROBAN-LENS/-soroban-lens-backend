-- Completed analyses. The full report is stored as JSONB; frequently filtered
-- fields are duplicated into columns so they can be indexed and aggregated
-- without parsing the document.
CREATE TABLE analyses (
    id                  UUID        PRIMARY KEY,
    transaction_hash    TEXT        NOT NULL,
    network             TEXT        NOT NULL,
    transaction_status  TEXT        NOT NULL,
    analysis_status     TEXT        NOT NULL,
    schema_version      TEXT        NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    report              JSONB       NOT NULL
);

CREATE INDEX analyses_created_at_idx ON analyses (created_at DESC);
CREATE INDEX analyses_transaction_idx ON analyses (transaction_hash, network);
