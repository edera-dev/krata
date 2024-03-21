#!/bin/sh
set -e

# shellcheck source-path=SCRIPTDIR source=common.sh
. "$(dirname "${0}")/common.sh"

if [ -z "${KRATA_KERNEL_BUILD_JOBS}" ]
then
  KRATA_KERNEL_BUILD_JOBS="2"
fi

TARGET_ARCH="$("${KRATA_DIR}/hack/build/arch.sh")"
BUNDLE_TAR="${OUTPUT_DIR}/bundle-systemd-${TARGET_ARCH}.tgz"
rm -f "${BUNDLE_TAR}"
BUNDLE_DIR="$(mktemp -d /tmp/krata-bundle.XXXXXXXXXXXXX)"
BUNDLE_DIR="${BUNDLE_DIR}/krata"
mkdir -p "${BUNDLE_DIR}"

./hack/build/cargo.sh build --release --bin kratad --bin kratanet --bin kratactl

RUST_TARGET="$(./hack/build/target.sh)"
for X in kratad kratanet kratactl
do
  cp "${KRATA_DIR}/target/${RUST_TARGET}/release/${X}" "${BUNDLE_DIR}/${X}"
done
./hack/initrd/build.sh
if [ "${KRATA_KERNEL_BUILD_SKIP}" != "1" ]
then
  ./hack/kernel/build.sh "-j${KRATA_KERNEL_BUILD_JOBS}"
fi

cd "${BUNDLE_DIR}"

cp "${KRATA_DIR}/target/initrd/initrd-${TARGET_ARCH}" initrd
cp "${KRATA_DIR}/target/kernel/kernel-${TARGET_ARCH}" kernel
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
