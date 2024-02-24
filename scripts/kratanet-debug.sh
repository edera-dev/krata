#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
# shellcheck source-path=krata-debug-common.sh
. "$(dirname "${REAL_SCRIPT}")/krata-debug-common.sh"

build_and_run kratanet "${@}"
