#!/bin/sh
set -e

# shellcheck source=common.sh
. "$(dirname "${0}")/common.sh"

if [ -z "${KRATA_KERNEL_BUILD_JOBS}" ]
then
  KRATA_KERNEL_BUILD_JOBS="2"
fi

BUNDLE_TAR="${OUTPUT_DIR}/bundle.tgz"
rm -f "${BUNDLE_TAR}"
BUNDLE_DIR="$(mktemp -d /tmp/krata-bundle.XXXXXXXXXXXXX)"
BUNDLE_DIR="${BUNDLE_DIR}/krata"
mkdir -p "${BUNDLE_DIR}"
for X in kratad kratanet kratactl
do
  ./scripts/build/cargo.sh build --release --bin "${X}"
  RUST_TARGET="$(./scripts/build/target.sh)"
  cp "${KRATA_DIR}/target/${RUST_TARGET}/release/${X}" "${BUNDLE_DIR}/${X}"
done
./scripts/initrd/build.sh
if [ "${KRATA_BUNDLE_SKIP_KERNEL_BUILD}" != "1" ]
then
  ./scripts/kernel/build.sh "-j${KRATA_KERNEL_BUILD_JOBS}"
fi

cd "${BUNDLE_DIR}"

cp "${KRATA_DIR}/target/initrd/initrd" initrd
cp "${KRATA_DIR}/target/kernel/kernel" kernel
cp "${KRATA_DIR}/resources/systemd/kratad.service" kratad.service
cp "${KRATA_DIR}/resources/systemd/kratanet.service" kratanet.service
cp "${KRATA_DIR}/resources/bundle/install.sh" install.sh
cp "${KRATA_DIR}/resources/bundle/uninstall.sh" uninstall.sh

for X in install.sh uninstall.sh kratactl kratad kratanet
do
  chmod +x "${X}"
done

cd ..
tar czf "${BUNDLE_TAR}" .
cd "${KRATA_DIR}"
rm -rf "$(dirname "${BUNDLE_DIR}")"
