#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="${PWD}"
OUTPUT_DIR="${KRATA_DIR}/target/dist"
mkdir -p "${OUTPUT_DIR}"
