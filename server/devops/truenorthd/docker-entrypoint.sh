#!/bin/sh

set -eu

network="${TRUENORTH_NETWORK:-main}"
rpc_allow_ip="${TRUENORTH_RPC_ALLOW_IP:-}"

case "$network" in
    main)
        ;;
    testnet4 | regtest | signet)
        set -- "-$network" "$@"
        ;;
    *)
        echo "Unsupported TrueNorth network: $network" >&2
        exit 1
        ;;
esac

if [ -n "$rpc_allow_ip" ]; then
    set -- -rpcbind=0.0.0.0 "-rpcallowip=$rpc_allow_ip" "$@"
fi

exec truenorthd \
    -datadir=/var/lib/truenorth \
    -server=1 \
    -printtoconsole=1 \
    -blockfilterindex=1 \
    "$@"
