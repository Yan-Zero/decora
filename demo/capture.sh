#!/usr/bin/env bash
set -u
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export DEMO_BASH="$(cygpath -w "$(command -v bash)")"
exec python -I "$SCRIPT_DIR/capture.py" "$@"
