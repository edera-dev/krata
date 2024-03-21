#!/bin/sh
set -e

# shellcheck source-path=SCRIPTDIR source=common.sh
. "$(dirname "${0}")/common.sh"

"${KRATA_DIR}/hack/dist/bundle.sh"

SYSTAR_VARIANT="systemd"
if [ "${KRATA_SYSTAR_OPENRC}" = "1" ]
then
  SYSTAR_VARIANT="openrc"
fi

TARGET_ARCH="$("${KRATA_DIR}/hack/build/arch.sh")"
SYSTAR="${OUTPUT_DIR}/system-${SYSTAR_VARIANT}-${TARGET_ARCH}.tgz"
rm -f "${SYSTAR}"
SYSTAR_DIR="$(mktemp -d /tmp/krata-systar.XXXXXXXXXXXXX)"
cd "${SYSTAR_DIR}"
tar xf "${OUTPUT_DIR}/bundle-systemd-${TARGET_ARCH}.tgz"

mkdir sys
cd sys

mkdir -p usr/bin usr/libexec
mv ../krata/kratactl usr/bin
mv ../krata/kratanet ../krata/kratad usr/libexec/

if [ "${SYSTAR_VARIANT}" = "openrc" ]
then
  mkdir -p etc/init.d
  cp "${KRATA_DIR}/resources/openrc/kratad" etc/init.d/kratad
  cp "${KRATA_DIR}/resources/openrc/kratanet" etc/init.d/kratanet
  chmod +x etc/init.d/kratad
  chmod +x etc/init.d/kratanet
else
  mkdir -p usr/lib/systemd/system
  mv ../krata/kratad.service ../krata/kratanet.service usr/lib/systemd/system/
fi

mkdir -p usr/share/krata/guest
mv ../krata/kernel ../krata/initrd usr/share/krata/guest

tar czf "${SYSTAR}" --owner 0 --group 0 .

cd "${KRATA_DIR}"
rm -rf "${SYSTAR_DIR}"
