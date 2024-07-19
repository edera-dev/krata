# Development Guide

## Structure

krata is composed of four major executables:

| Executable | Runs On | User Interaction | Dev Runner               | Code Path         |
| ---------- | ------- | ---------------- | ------------------------ | ----------------- |
| kratad     | host    | backend daemon   | ./hack/debug/kratad.sh   | crates/daemon     |
| kratanet   | host    | backend daemon   | ./hack/debug/kratanet.sh | crates/network    |
| kratactl   | host    | CLI tool         | ./hack/debug/kratactl.sh | crates/ctl        |
| kratazone  | zone    | none, zone init  | N/A                      | crates/zone       |

You will find the code to each executable available in the bin/ and src/ directories inside
it's corresponding code path from the above table.

## Environment

| Component     | Specification | Notes                                                                             |
| ------------- | ------------- | --------------------------------------------------------------------------------- |
| Architecture  | x86_64        | aarch64 support is still in development                                           |
| Memory        | At least 6GB  | dom0 will need to be configured with lower memory limit to give krata zones room  | 
| Xen           | 4.17          | Temporary due to hardcoded interface version constants                            |
| Debian        | stable / sid  | Debian is recommended due to the ease of Xen setup                                |
| rustup        | any           | Install Rustup from https://rustup.rs                                             |

## Setup Guide

1. Install the specified Debian version on a x86_64 host _capable_ of KVM (NOTE: KVM is not used, Xen is a type-1 hypervisor).

2. Install required packages:

```sh
$ apt install git xen-system-amd64 build-essential \
      libclang-dev musl-tools flex bison libelf-dev libssl-dev bc \
      protobuf-compiler libprotobuf-dev squashfs-tools erofs-utils
```

3. Install [rustup](https://rustup.rs) for managing a Rust environment.

Make sure to install the targets that you need for krata:

```sh
$ rustup target add x86_64-unknown-linux-gnu
$ rustup target add x86_64-unknown-linux-musl
```

4. Configure `/etc/default/grub.d/xen.cfg` to give krata zones some room:

```sh
# Configure dom0_mem to be 4GB, but leave the rest of the RAM for krata zones.
GRUB_CMDLINE_XEN_DEFAULT="dom0_mem=4G,max:4G"
```

After changing the grub config, update grub: `update-grub`

Then reboot to boot the system as a Xen dom0.

You can validate that Xen is setup by running `dmesg | grep "Hypervisor detected"` and ensuring it returns a line like `Hypervisor detected: Xen PV`, if that is missing, the host is not running under Xen.

5. Clone the krata source code:
```sh
$ git clone https://github.com/edera-dev/krata.git krata
$ cd krata
```

6. Fetch the zone kernel image:

```sh
$ ./hack/kernel/fetch.sh -u
```

7. Copy the zone kernel artifacts to `/var/lib/krata/zone/kernel` so it is automatically detected by kratad:

```sh
$ mkdir -p /var/lib/krata/zone
$ cp target/kernel/kernel-x86_64 /var/lib/krata/zone/kernel
$ cp target/kernel/addons-x86_64.squashfs /var/lib/krata/zone/addons.squashfs
```

8. Launch `./hack/debug/kratad.sh` and keep it running in the foreground.
9. Launch `./hack/debug/kratanet.sh` and keep it running in the foreground.
10. Run `kratactl` to launch a zone:

```sh
$ ./hack/debug/kratactl.sh launch --attach alpine:latest
```

To detach from the zone console, use `Ctrl + ]` on your keyboard.

To list the running zones, run:
```sh
$ ./hack/debug/kratactl.sh list
```

To destroy a running zone, copy it's UUID from either the launch command or the zone list and run:
```sh
$ ./hack/debug/kratactl.sh destroy ZONE_UUID
```
