#!/bin/sh
set -e

apk add --update-cache grub-efi
grub-install --target=x86_64-efi --efi-directory=/boot/efi --no-nvram --skip-fs-probe --bootloader-id=BOOT
mv /boot/efi/EFI/BOOT/grubx64.efi /boot/efi/EFI/BOOT/BOOTX64.efi

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
