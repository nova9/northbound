CREATE TABLE index_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    network TEXT NOT NULL CHECK (network IN ('main', 'regtest')),
    genesis_hash BLOB NOT NULL CHECK (length(genesis_hash) = 32)
) STRICT;

CREATE TABLE blocks (
    hash BLOB PRIMARY KEY NOT NULL CHECK (length(hash) = 32),
    height INTEGER UNIQUE NOT NULL CHECK (height >= 0),
    previous_hash BLOB CHECK (length(previous_hash) = 32),
    timestamp INTEGER NOT NULL CHECK (timestamp >= 0),
    CHECK ((height = 0 AND previous_hash IS NULL)
        OR (height > 0 AND previous_hash IS NOT NULL))
) STRICT;

CREATE TABLE transactions (
    txid BLOB PRIMARY KEY NOT NULL CHECK (length(txid) = 32),
    block_hash BLOB NOT NULL REFERENCES blocks(hash) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    is_coinbase INTEGER NOT NULL CHECK (is_coinbase IN (0, 1)),
    UNIQUE (block_hash, position)
) STRICT;

CREATE TABLE outputs (
    txid BLOB NOT NULL REFERENCES transactions(txid) ON DELETE CASCADE,
    vout INTEGER NOT NULL CHECK (vout BETWEEN 0 AND 4294967295),
    amount INTEGER NOT NULL CHECK (amount BETWEEN 0 AND 1000000000000000000),
    script BLOB NOT NULL,
    utxo_eligible INTEGER NOT NULL CHECK (utxo_eligible IN (0, 1)),
    PRIMARY KEY (txid, vout)
) STRICT;
CREATE INDEX outputs_by_script ON outputs(script);

CREATE TABLE spends (
    spending_txid BLOB NOT NULL REFERENCES transactions(txid) ON DELETE CASCADE,
    vin INTEGER NOT NULL CHECK (vin BETWEEN 0 AND 4294967295),
    previous_txid BLOB NOT NULL,
    previous_vout INTEGER NOT NULL CHECK (previous_vout BETWEEN 0 AND 4294967295),
    PRIMARY KEY (spending_txid, vin),
    UNIQUE (previous_txid, previous_vout),
    FOREIGN KEY (previous_txid, previous_vout)
        REFERENCES outputs(txid, vout) ON DELETE NO ACTION
) STRICT;

PRAGMA application_id = 0x4e425849;
PRAGMA user_version = 1;
