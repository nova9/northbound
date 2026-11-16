#!/usr/bin/env sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

export K6_WEB_DASHBOARD=true
export K6_WEB_DASHBOARD_OPEN=true

exec k6 run --no-usage-report "$@" "$script_directory/load-api.js"
