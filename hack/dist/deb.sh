#!/bin/sh
set -e

# shellcheck source-path=SCRIPTDIR source=common.sh
. "$(dirname "${0}")/common.sh"

"${KRATA_DIR}/hack/dist/systar.sh"

KRATA_VERSION="$("${KRATA_DIR}/hack/dist/version.sh")"
TARGET_ARCH_STANDARD="$(KRATA_ARCH_ALT_NAME=0 "${KRATA_DIR}/hack/build/arch.sh")"
TARGET_ARCH_DEBIAN="$(KRATA_ARCH_ALT_NAME=1 "${KRATA_DIR}/hack/build/arch.sh")"

cd "${OUTPUT_DIR}"

rm -f "krata_${KRATA_VERSION}_${TARGET_ARCH_DEBIAN}.deb"

fpm -s tar -t deb \
  --name krata \
  --license agpl3 \
  --version "${KRATA_VERSION}" \
  --architecture "${TARGET_ARCH_DEBIAN}" \
  --depends "xen-system-${TARGET_ARCH_DEBIAN}" \
  --description "Krata Hypervisor" \
  --url "https://krata.dev" \
  --maintainer "Edera Team <contact@edera.dev>" \
  -x "usr/lib/**" \
  --deb-systemd "${KRATA_DIR}/resources/systemd/kratad.service" \
  --deb-systemd "${KRATA_DIR}/resources/systemd/kratanet.service" \
  --deb-systemd-enable \
  --deb-systemd-auto-start \
  "${OUTPUT_DIR}/system-systemd-${TARGET_ARCH_STANDARD}.tgz"
