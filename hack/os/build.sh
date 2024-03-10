#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."

./hack/dist/apk.sh
KRATA_VERSION="$(./hack/dist/version.sh)"
TARGET_ARCH="$(./hack/build/arch.sh)"

TARGET_DIR="${PWD}/target"
TARGET_OS_DIR="${TARGET_DIR}/os"
mkdir -p "${TARGET_OS_DIR}"
cp "${TARGET_DIR}/dist/krata_${KRATA_VERSION}_${TARGET_ARCH}.apk" "${TARGET_OS_DIR}/krata.apk"
docker run --rm --privileged -v "${PWD}:/mnt" -it alpine:latest "/mnt/os/internal/stage1.sh"
sudo chown "${USER}:${GROUP}" "${TARGET_OS_DIR}/rootfs.tgz"
sudo modprobe nbd

next_nbd_device() {
  find /dev -maxdepth 2 -name 'nbd[0-9]*' | while read -r DEVICE
  do
    if [ "$(sudo blockdev --getsize64 "${DEVICE}")" = "0" ]
    then
      echo "${DEVICE}"
      break
    fi
  done
}

NBD_DEVICE="$(next_nbd_device)"

if [ -z "${NBD_DEVICE}" ]
then
  echo "ERROR: unable to allocate nbd device" > /dev/stderr
  exit 1
fi

OS_IMAGE="${TARGET_OS_DIR}/krata.qcow2"
EFI_PART="${NBD_DEVICE}p1"
ROOT_PART="${NBD_DEVICE}p2"
ROOT_DIR="${TARGET_OS_DIR}/root"
EFI_DIR="${ROOT_DIR}/boot/efi"

cleanup() {
  trap '' EXIT HUP INT TERM
  sudo umount -R "${ROOT_DIR}" > /dev/null 2>&1 || true
  sudo umount "${EFI_PART}" > /dev/null 2>&1 || true
  sudo umount "${ROOT_PART}" > /dev/null 2>&1 || true
  sudo qemu-nbd --disconnect "${NBD_DEVICE}" > /dev/null 2>&1 || true
  sudo rm -rf "${ROOT_DIR}"
}

rm -f "${OS_IMAGE}"
qemu-img create -f qcow2 "${TARGET_OS_DIR}/krata.qcow2" "2G"

trap cleanup EXIT HUP INT TERM
sudo qemu-nbd --connect="${NBD_DEVICE}" --cache=writeback -f qcow2 "${OS_IMAGE}"
printf '%s\n' \
			 'label: gpt' \
			 'name=efi,type=U,size=128M,bootable' \
			 'name=system,type=L' | sudo sfdisk "${NBD_DEVICE}"
sudo mkfs.fat -F32 -n EFI "${EFI_PART}"
sudo mkfs.ext4 -L root -E discard "${ROOT_PART}"

mkdir -p "${ROOT_DIR}"

sudo mount -t ext4 "${ROOT_PART}" "${ROOT_DIR}"
sudo mkdir -p "${EFI_DIR}"
sudo mount -t vfat "${EFI_PART}" "${EFI_DIR}"

sudo tar xf "${TARGET_OS_DIR}/rootfs.tar" -C "${ROOT_DIR}"
ROOT_UUID="$(sudo blkid "${ROOT_PART}" | sed -En 's/.*\bUUID="([^"]+)".*/\1/p')"
EFI_UUID="$(sudo blkid "${EFI_PART}" | sed -En 's/.*\bUUID="([^"]+)".*/\1/p')"
echo "${ROOT_UUID}"

sudo mkdir -p "${ROOT_DIR}/proc" "${ROOT_DIR}/dev" "${ROOT_DIR}/sys"
sudo mount -t proc none "${ROOT_DIR}/proc"
sudo mount --bind /dev "${ROOT_DIR}/dev"
sudo mount --make-private "${ROOT_DIR}/dev"
sudo mount --bind /sys "${ROOT_DIR}/sys"
sudo mount --make-private "${ROOT_DIR}/sys"

sudo cp "${PWD}/os/internal/stage2.sh" "${ROOT_DIR}/stage2.sh"
echo "${ROOT_UUID}" | sudo tee "${ROOT_DIR}/root-uuid" > /dev/null
sudo chroot "${ROOT_DIR}" /bin/sh -c "/stage2.sh"
sudo rm -f "${ROOT_DIR}/stage2.sh"
sudo rm -f "${ROOT_DIR}/root-uuid"

{
  echo "# krata fstab"
  echo "UUID=${ROOT_UUID} / ext4 relatime 0 1"
  echo "UUID=${EFI_UUID} / vfat rw,relatime,fmask=0133,codepage=437,iocharset=ascii,shortname=mixed,utf8,errors=remount-ro 0 2"
} | sudo tee "${ROOT_DIR}/etc/fstab" > /dev/null

cleanup

OS_SMALL_IMAGE="${TARGET_OS_DIR}/krata.small.qcow2"
qemu-img convert -O qcow2 "${OS_IMAGE}" "${OS_SMALL_IMAGE}"
mv -f "${OS_SMALL_IMAGE}" "${OS_IMAGE}"
