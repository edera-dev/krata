#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."

find hack -type f -name '*.sh' -print0 | xargs -0 shellcheck -x
find os/internal -type f -name '*.sh' -print0 | xargs -0 shellcheck -x
