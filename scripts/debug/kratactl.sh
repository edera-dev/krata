#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
DEBUG_DIR="$(dirname "${REAL_SCRIPT}")"
# shellcheck source=common.sh
. "${DEBUG_DIR}/common.sh"

build_and_run kratactl "${@}"
