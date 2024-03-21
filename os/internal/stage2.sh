#!/bin/sh
set -e

TARGET_ARCH="${1}"
TARGET_ARCH_ALT="${2}"
apk add --update-cache grub-efi
grub-install --target="${TARGET_ARCH_ALT}-efi" --efi-directory=/boot/efi --no-nvram --skip-fs-probe --bootloader-id=BOOT

FROM_EFI_FILE="grubx64.efi"
TO_EFI_FILE="BOOTX64.efi"
if [ "${TARGET_ARCH}" = "aarch64" ]
then
  FROM_EFI_FILE="grubaa64.efi"
  TO_EFI_FILE="BOOTA64.efi"
fi

mv "/boot/efi/EFI/BOOT/${FROM_EFI_FILE}" "/boot/efi/EFI/BOOT/${TO_EFI_FILE}"

ROOT_UUID="$(cat /root-uuid)"

{
    echo 'GRUB_CMDLINE_XEN_DEFAULT="dom0_mem=1024M,max:1024M"'
    echo "GRUB_CMDLINE_LINUX_DEFAULT=\"quiet rootfstype=ext4 root=UUID=${ROOT_UUID} modules=ext4\""
    echo 'GRUB_DEFAULT="saved"'
    echo 'GRUB_SAVEDEFAULT="true"'
} >> /etc/default/grub

grub-mkconfig -o /boot/grub/grub.cfg
grub-set-default "$(grep ^menuentry /boot/grub/grub.cfg | grep Xen | cut -d \' -f 2 | head -1)"
rm -rf /var/cache/apk/*
