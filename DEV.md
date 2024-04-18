# Development Guide

## Structure

krata is composed of four major executables:

| Executable | Runs On | User Interaction | Dev Runner               | Code Path         |
| ---------- | ------- | ---------------- | ------------------------ | ----------------- |
| kratad     | host    | backend daemon   | ./hack/debug/kratad.sh   | crates/daemon     |
| kratanet   | host    | backend daemon   | ./hack/debug/kratanet.sh | crates/network    |
| kratactl   | host    | CLI tool         | ./hack/debug/kratactl.sh | crates/ctl        |
| krataguest | guest   | none, guest init | N/A                      | crates/guest      |

You will find the code to each executable available in the bin/ and src/ directories inside
it's corresponding code path from the above table.

## Environment

| Component     | Specification | Notes                                                                             |
| ------------- | ------------- | --------------------------------------------------------------------------------- |
| Architecture  | x86_64        | aarch64 support is still in development                                           |
| Memory        | At least 6GB  | dom0 will need to be configured will lower memory limit to give krata guests room | 
| Xen           | 4.17          | Temporary due to hardcoded interface version constants                            |
| Debian        | stable / sid  | Debian is recommended due to the ease of Xen setup                                |
| rustup        | any           | Install Rustup from https://rustup.rs                                             |

## Setup Guide

1. Install the specified Debian version on a x86_64 host _capable_ of KVM (NOTE: KVM is not used, Xen is a type-1 hypervisor).

2. Install required packages:

```sh
$ apt install git xen-system-amd64 build-essential libclang-dev musl-tools flex bison libelf-dev libssl-dev bc protobuf-compiler libprotobuf-dev squashfs-tools erofs-utils
```

3. Install [rustup](https://rustup.rs) for managing a Rust environment.

Make sure to install the targets that you need for krata:

```sh
$ rustup target add x86_64-unknown-linux-gnu
$ rustup target add x86_64-unknown-linux-musl
```

4. Configure `/etc/default/grub.d/xen.cfg` to give krata guests some room:

```sh
# Configure dom0_mem to be 4GB, but leave the rest of the RAM for krata guests.
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

6. Build a guest kernel image:

```sh
$ ./hack/kernel/build.sh
```

7. Copy the guest kernel image at `target/kernel/kernel-x86_64` to `/var/lib/krata/guest/kernel` to have it automatically detected by kratad.
8. Launch `./hack/debug/kratad.sh` and keep it running in the foreground.
9. Launch `./hack/debug/kratanet.sh` and keep it running in the foreground.
10. Run kratactl to launch a guest:

```sh
$ ./hack/debug/kratactl.sh launch --attach alpine:latest
```

To detach from the guest console, use `Ctrl + ]` on your keyboard.

To list the running guests, run:
```sh
$ ./hack/debug/kratactl.sh list
```

To destroy a running guest, copy it's UUID from either the launch command or the guest list and run:
```sh
$ ./hack/debug/kratactl.sh destroy GUEST_UUID
```
