#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/.."
KRATA_DIR="${PWD}"

OUTPUT_DIR="${KRATA_DIR}/target/bundle"
mkdir -p "${OUTPUT_DIR}"
BUNDLE_TAR="${OUTPUT_DIR}/krata.tgz"
rm -f "${BUNDLE_TAR}"
BUNDLE_DIR="$(mktemp -d /tmp/krata-bundle.XXXXXXXXXXXXX)"
BUNDLE_DIR="${BUNDLE_DIR}/krata"
mkdir -p "${BUNDLE_DIR}"
for X in kratad kratanet kratactl
do
  cargo build --release --target x86_64-unknown-linux-gnu --bin "${X}"
  cp "${KRATA_DIR}/target/x86_64-unknown-linux-gnu/release/${X}" "${BUNDLE_DIR}/${X}"
done
./initrd/build.sh
if [ "${KRATA_BUNDLE_SKIP_KERNEL_BUILD}" != "1" ]
then
  ./kernel/build.sh
fi
cd "${BUNDLE_DIR}"
cp "${KRATA_DIR}/initrd/target/initrd" initrd
cp "${KRATA_DIR}/kernel/target/kernel" kernel
cp "${KRATA_DIR}/resources/systemd/kratad.service" kratad.service
cp "${KRATA_DIR}/resources/systemd/kratanet.service" kratanet.service
cp "${KRATA_DIR}/resources/install/install.sh" install.sh

for X in install.sh kratactl kratad kratanet
do
  chmod +x "${X}"
done

cd ..
tar czf "${BUNDLE_TAR}" .
cd "${KRATA_DIR}"
rm -rf "$(dirname "${BUNDLE_DIR}")"
