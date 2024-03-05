#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="${PWD}"
NIGHTLY_TAR="${KRATA_DIR}/krata.tgz"
NIGHTLY_DIR="$(mktemp -d /tmp/krata-release.XXXXXXXXXXXXX)"
for X in kratad kratanet kratactl
do
  cargo build --release --target x86_64-unknown-linux-gnu --bin "${X}"
  cp "${KRATA_DIR}/target/x86_64-unknown-linux-gnu/release/${X}" "${NIGHTLY_DIR}/${X}"

done
./initrd/build.sh
./kernel/build.sh
cd "${NIGHTLY_DIR}"
cp "${KRATA_DIR}/initrd/target/initrd" initrd
cp "${KRATA_DIR}/kernel/target/kernel" kernel
tar czf "${NIGHTLY_TAR}" .
cd "${KRATA_DIR}"
rm -rf "${NIGHTLY_DIR}"
