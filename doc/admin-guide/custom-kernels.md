Custom Kernels in krata
=======================

Krata supports using custom kernels instead of the default Edera-provided
kernel both on a system-wide and zone-wide basis.  Krata also supports using
a custom host kernel, as long as it meets certain technical requirements.

System-wide default kernel for zones
------------------------------------

The standard system-wide default kernel for zones is stored in
`/var/lib/krata/zone/kernel` which is the kernel image that should be
booted for the zone, and `/var/lib/krata/zone/addons.squashfs`,
which contains a set of kernel modules that should be mounted in the
zone.

Zone-wide alternative kernels via OCI
-------------------------------------

Krata also supports fetching alternative kernel images for use in zones
via OCI repositories.  These kernel images are distributed like any other
OCI image, but are not intended to be directly executed by an OCI runtime.

To select an alternative kernel, you can supply the `-k` option to the
`kratactl zone launch` command that specifies an OCI tag to pull the
alternative kernel image from.

OCI-based kernel image contents
-------------------------------

OCI-based kernel images contain the following files:

* `/kernel/image`: The kernel image itself.

* `/kernel/addons.squashfs`: A squashfs file containing the kernel
  modules for a given kernel image.

* `/kernel/metadata`: A file containing the following metadata fields
  in `KEY=VALUE` format:
    - `KERNEL_ARCH`: The kernel architecture (`x86_64` or `aarch64`)
    - `KERNEL_VERSION`: The kernel version
    - `KERNEL_FLAVOR`: The kernel flavor (examples: `standard`, `dom0` or `openpax`)
    - `KERNEL_CONFIG`: The digest for the relevant configuration file stored in the OCI
      repository
    - `KERNEL_TAGS`: The OCI tags this kernel image was originally built for
      (example: `latest`)

Minimum requirements for a zone-wide/system-wide kernel
-------------------------------------------------------

The following configuration options must be set:

```
CONFIG_XEN=y
CONFIG_XEN_PV=y
CONFIG_XEN_512GB=y
CONFIG_XEN_PV_SMP=y
CONFIG_XEN_PVHVM=y
CONFIG_XEN_PVHVM_SMP=y
CONFIG_XEN_PVHVM_GUEST=y
CONFIG_XEN_SAVE_RESTORE=y
CONFIG_XEN_PVH=y
CONFIG_XEN_PV_MSR_SAFE=y
CONFIG_PCI_XEN=y
CONFIG_NET_9P_XEN=y
CONFIG_XEN_PCIDEV_FRONTEND=y
CONFIG_XEN_BLKDEV_FRONTEND=y
CONFIG_XEN_NETDEV_FRONTEND=y
CONFIG_INPUT_XEN_KBDDEV_FRONTEND=y
CONFIG_HVC_XEN=y
CONFIG_HVC_XEN_FRONTEND=y
CONFIG_XEN_FBDEV_FRONTEND=m
CONFIG_XEN_BALLOON=y
CONFIG_XEN_BALLOON_MEMORY_HOTPLUG=y
CONFIG_XEN_MEMORY_HOTPLUG_LIMIT=512
CONFIG_XEN_SCRUB_PAGES_DEFAULT=y
CONFIG_XEN_DEV_EVTCHN=y
CONFIG_XEN_BACKEND=y
CONFIG_XENFS=y
CONFIG_XEN_COMPAT_XENFS=y
CONFIG_XEN_SYS_HYPERVISOR=y
CONFIG_XEN_XENBUS_FRONTEND=y
CONFIG_SWIOTLB_XEN=y
CONFIG_XEN_HAVE_PVMMU=y
CONFIG_XEN_EFI=y
CONFIG_XEN_AUTO_XLATE=y
CONFIG_XEN_ACPI=y
CONFIG_XEN_HAVE_VPMU=y
CONFIG_XEN_GRANT_DMA_OPS=y
CONFIG_XEN_VIRTIO=y
```

It is possible to copy these options into a `.config` file and then use
`make olddefconfig` to build the rest of the kernel configuration, which
you can then use to build a kernel as desired.

The [linux-kernel-oci][edera-linux-kernel-oci] repository provides some example configurations
and can generate a Dockerfile which will build a kernel image.

   [edera-linux-kernel-oci]: https://github.com/edera-dev/linux-kernel-oci

Minimum requirements for a host kernel
--------------------------------------

The configuration options above are also required for a host kernel.
In addition, the following options are also required:

```
CONFIG_XEN_PV_DOM0=y
CONFIG_XEN_DOM0=y
CONFIG_PCI_XEN=y
CONFIG_XEN_PCIDEV_BACKEND=y
CONFIG_XEN_BLKDEV_BACKEND=y
CONFIG_XEN_NETDEV_BACKEND=y
CONFIG_XEN_SCSI_BACKEND=y
CONFIG_XEN_PVCALLS_BACKEND=y
CONFIG_TCG_XEN=m
CONFIG_XEN_WDT=y
CONFIG_XEN_DEV_EVTCHN=y
CONFIG_XEN_GNTDEV=y
CONFIG_XEN_GRANT_DEV_ALLOC=y
CONFIG_XEN_GRANT_DMA_ALLOC=y
CONFIG_SWIOTLB_XEN=y
CONFIG_XEN_PRIVCMD=y
CONFIG_XEN_ACPI_PROCESSOR=y
CONFIG_XEN_MCE_LOG=y
```

Build and install the kernel as you normally would for your system.
Assuming GRUB is the bootloader, it will automatically detect the new
host kernel when you run `grub-mkconfig` or `grub2-mkconfig`.
