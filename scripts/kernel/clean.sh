#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="${PWD}"
KERNEL_DIR="${KRATA_DIR}/kernel"

# shellcheck disable=SC2010
for ITEM in $(ls "${KERNEL_DIR}" | grep "^linux-")
do
  rm -rf "${KERNEL_DIR:?}/${ITEM}"
done
