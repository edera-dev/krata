#!/bin/sh
set -e

# shellcheck source-path=SCRIPTDIR source=common.sh
. "$(dirname "${0}")/common.sh"

export TARGET_LIBC="musl"
KRATA_SYSTAR_OPENRC=1 "${KRATA_DIR}/hack/dist/systar.sh"

KRATA_VERSION="$("${KRATA_DIR}/hack/dist/version.sh")"
TARGET_ARCH="$("${KRATA_DIR}/hack/build/arch.sh")"

cd "${OUTPUT_DIR}"

rm -f "krata_${KRATA_VERSION}_${TARGET_ARCH}.apk"

fpm -s tar -t apk \
  --name krata \
  --license agpl3 \
  --version "${KRATA_VERSION}" \
  --architecture "${TARGET_ARCH}" \
  --description "Krata Hypervisor" \
  --url "https://krata.dev" \
  --maintainer "Edera Team <contact@edera.dev>" \
  "${OUTPUT_DIR}/system-openrc-${TARGET_ARCH}.tgz"
