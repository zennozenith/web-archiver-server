#!/usr/bin/env bash
# Enforce the downward dependency flow described in the design document.
#
# `transport -> service -> db -> infra -> util`, with `identity`, `domain` and
# `error` as the shared kernel. The compiler cannot reject an upward `use`
# inside one crate, so this script makes the most important edges mechanical.
set -euo pipefail

cd "$(dirname "$0")/.."

status=0

check_forbidden() {
    local layer="$1"
    local imports="$2"
    local matches

    if matches=$(grep -RInE --include='*.rs' "crate::(${imports})" "src/${layer}" 2>/dev/null); then
        printf '%s\n' "error: ${layer} must not import ${imports}:" >&2
        printf '%s\n' "$matches" >&2
        status=1
    fi
}

# Persistence and adapters sit below the use-case and HTTP layers.
check_forbidden db 'service|transport'
check_forbidden infra 'db|service|transport'

exit "$status"
