#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
# shellcheck source-path=krata-debug-common.sh
. "$(dirname "${REAL_SCRIPT}")/krata-debug-common.sh"

KRATA_BUILD_INITRD=1 build_and_run kratad "${@}"
