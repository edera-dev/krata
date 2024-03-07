#!/bin/sh
set -e

# shellcheck source-path=SCRIPTDIR source=common.sh
. "$(dirname "${0}")/common.sh"
cd "${KRATA_DIR}"

KRATA_VERSION="$(grep -A1 -F '[workspace.package]' Cargo.toml | grep 'version' | awk '{print $3}' | sed 's/"//g')"
if [ -z "${KRATA_VERSION}" ]
then
  echo "ERROR: failed to determine krata version" > /dev/stderr
  exit 1
fi

echo "${KRATA_VERSION}"
