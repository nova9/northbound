# TrueNorth RPC client

Shared by the indexer and API server. `NodeClient` reads cookie authentication
for each request, disables redirects, uses 30-second timeouts, and bounds JSON
responses to 64 MiB. Errors distinguish transport failures, daemon error codes,
invalid response data, and cookie I/O without logging credentials or bodies.

The crate decodes exact decimal amounts into integer atomic units and provides
block, pending transaction, fee estimate, transaction acceptance, broadcast, and
transaction lookup methods. It contains no database, wallet ownership, HTTP API,
or blockchain scanning logic. The daemon validates consensus.
