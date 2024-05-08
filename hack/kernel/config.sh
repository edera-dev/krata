#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="$(realpath "${PWD}")"

# shellcheck source-path=SCRIPTDIR source=common.sh
. "${KRATA_DIR}/hack/kernel/common.sh"

rm -rf "${MODULES_INSTALL_PATH}"
rm -rf "${ADDONS_OUTPUT_PATH}"
rm -rf "${ADDONS_SQUASHFS_PATH}"

make -C "${KERNEL_SRC}" ARCH="${TARGET_ARCH_KERNEL}" "${CROSS_COMPILE_MAKE}" INSTALL_MOD_PATH="${MODULES_INSTALL_PATH}" nconfig
cp "${KERNEL_SRC}/.config" "${KERNEL_CONFIG_FILE}"
