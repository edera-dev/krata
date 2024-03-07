#!/bin/sh
set -e

# shellcheck source=common.sh
. "$(dirname "${0}")/common.sh"

"${KRATA_DIR}/scripts/dist/systar.sh"

KRATA_VERSION="$("${KRATA_DIR}/scripts/dist/version.sh")"
TARGET_ARCH="$(KRATA_ARCH_ALT_NAME=1 "${KRATA_DIR}/scripts/build/arch.sh")"

cd "${OUTPUT_DIR}"

rm -f "krata_${KRATA_VERSION}_${TARGET_ARCH}.deb"

fpm -s tar -t deb \
  --name krata \
  --license agpl3 \
  --version "${KRATA_VERSION}" \
  --architecture "${TARGET_ARCH}" \
  --depends "xen-system-${TARGET_ARCH}" \
  --description "Krata Hypervisor" \
  --url "https://krata.dev" \
  --maintainer "Edera Team <contact@edera.dev>" \
  -x "usr/lib/**" \
  --deb-systemd "${KRATA_DIR}/resources/systemd/kratad.service" \
  --deb-systemd "${KRATA_DIR}/resources/systemd/kratanet.service" \
  --deb-systemd-enable \
  --deb-systemd-auto-start \
  "${OUTPUT_DIR}/system.tgz"
