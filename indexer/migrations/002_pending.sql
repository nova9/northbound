CREATE TABLE sync_status (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    observed_height INTEGER CHECK (observed_height >= 0),
    observed_hash BLOB CHECK (length(observed_hash) = 32),
    last_node_contact INTEGER,
    ready INTEGER NOT NULL DEFAULT 0 CHECK (ready IN (0, 1)),
    pending_base_hash BLOB CHECK (length(pending_base_hash) = 32),
    pending_sequence INTEGER,
    pending_revision INTEGER NOT NULL DEFAULT 0 CHECK (pending_revision >= 0),
    pending_observed_at INTEGER
) STRICT;
INSERT INTO sync_status (singleton) VALUES (1);

CREATE TABLE mempool_transactions (
    txid BLOB PRIMARY KEY NOT NULL CHECK (length(txid) = 32),
    first_seen_at INTEGER NOT NULL CHECK (first_seen_at >= 0)
) STRICT;

CREATE TABLE mempool_outputs (
    txid BLOB NOT NULL REFERENCES mempool_transactions(txid) ON DELETE CASCADE,
    vout INTEGER NOT NULL CHECK (vout BETWEEN 0 AND 4294967295),
    amount INTEGER NOT NULL CHECK (amount BETWEEN 0 AND 1000000000000000000),
    script BLOB NOT NULL,
    utxo_eligible INTEGER NOT NULL CHECK (utxo_eligible IN (0, 1)),
    PRIMARY KEY (txid, vout)
) STRICT;
CREATE INDEX mempool_outputs_by_script ON mempool_outputs(script);

CREATE TABLE mempool_spends (
    spending_txid BLOB NOT NULL REFERENCES mempool_transactions(txid) ON DELETE CASCADE,
    vin INTEGER NOT NULL CHECK (vin BETWEEN 0 AND 4294967295),
    previous_txid BLOB NOT NULL CHECK (length(previous_txid) = 32),
    previous_vout INTEGER NOT NULL CHECK (previous_vout BETWEEN 0 AND 4294967295),
    PRIMARY KEY (spending_txid, vin),
    UNIQUE (previous_txid, previous_vout)
) STRICT;

CREATE VIEW all_outputs AS
    SELECT txid, vout, amount, script, utxo_eligible FROM outputs
    UNION ALL SELECT txid, vout, amount, script, utxo_eligible FROM mempool_outputs;
CREATE VIEW all_spends AS
    SELECT spending_txid, vin, previous_txid, previous_vout FROM spends
    UNION ALL SELECT spending_txid, vin, previous_txid, previous_vout FROM mempool_spends;
CREATE VIEW all_transactions AS
    SELECT transactions.txid, blocks.height AS block_height, blocks.timestamp, transactions.is_coinbase
    FROM transactions JOIN blocks ON blocks.hash = transactions.block_hash
    UNION ALL SELECT txid, NULL, first_seen_at, 0 FROM mempool_transactions;

PRAGMA user_version = 2;
