#!/bin/sh
set -e

# shellcheck source-path=SCRIPTDIR source=common.sh
. "$(dirname "${0}")/common.sh"

"${KRATA_DIR}/hack/dist/bundle.sh"

SYSTAR="${OUTPUT_DIR}/system.tgz"
rm -f "${SYSTAR}"
SYSTAR_DIR="$(mktemp -d /tmp/krata-systar.XXXXXXXXXXXXX)"
cd "${SYSTAR_DIR}"
tar xf "${OUTPUT_DIR}/bundle.tgz"

mkdir sys
cd sys

mkdir -p usr/bin usr/libexec
mv ../krata/kratactl usr/bin
mv ../krata/kratanet ../krata/kratad usr/libexec/

mkdir -p usr/lib/systemd/system
mv ../krata/kratad.service ../krata/kratanet.service usr/lib/systemd/system/

mkdir -p usr/share/krata/guest
mv ../krata/kernel ../krata/initrd usr/share/krata/guest

tar czf "${SYSTAR}" --owner 0 --group 0 .

cd "${KRATA_DIR}"
rm -rf "${SYSTAR_DIR}"
