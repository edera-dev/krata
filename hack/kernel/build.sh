#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="$(realpath "${PWD}")"

# shellcheck source-path=SCRIPTDIR source=common.sh
. "${KRATA_DIR}/hack/kernel/common.sh"

make -C "${KERNEL_SRC}" ARCH="${TARGET_ARCH_KERNEL}" -j"${KRATA_KERNEL_BUILD_JOBS}" "${CROSS_COMPILE_MAKE}" "${IMAGE_TARGET}" modules

rm -rf "${MODULES_INSTALL_PATH}"
rm -rf "${ADDONS_OUTPUT_PATH}"
rm -rf "${ADDONS_SQUASHFS_PATH}"

make -C "${KERNEL_SRC}" ARCH="${TARGET_ARCH_KERNEL}" -j"${KRATA_KERNEL_BUILD_JOBS}" "${CROSS_COMPILE_MAKE}" INSTALL_MOD_PATH="${MODULES_INSTALL_PATH}" modules_install
KERNEL_MODULES_VER="$(ls "${MODULES_INSTALL_PATH}/lib/modules")"

mkdir -p "${ADDONS_OUTPUT_PATH}"
mv "${MODULES_INSTALL_PATH}/lib/modules/${KERNEL_MODULES_VER}" "${MODULES_OUTPUT_PATH}"
rm -rf "${MODULES_INSTALL_PATH}"
[ -L "${MODULES_OUTPUT_PATH}/build" ] && unlink "${MODULES_OUTPUT_PATH}/build"

mksquashfs "${ADDONS_OUTPUT_PATH}" "${ADDONS_SQUASHFS_PATH}" -all-root

if [ "${TARGET_ARCH_STANDARD}" = "x86_64" ]
then
  cp "${KERNEL_SRC}/arch/x86/boot/bzImage" "${OUTPUT_DIR}/kernel-${TARGET_ARCH_STANDARD}"
elif [ "${TARGET_ARCH_STANDARD}" = "aarch64" ]
then
  cp "${KERNEL_SRC}/arch/arm64/boot/Image.gz" "${OUTPUT_DIR}/kernel-${TARGET_ARCH_STANDARD}"
else
  echo "ERROR: unable to determine what file is the vmlinuz for ${TARGET_ARCH_STANDARD}" > /dev/stderr
  exit 1
fi
