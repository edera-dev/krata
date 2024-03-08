# Development Guide

## Structure

krata is composed of three major executables:

| Executable | Runs On | User Interaction | Dev Runner               | Code Path         |
| ---------- | ------- | ---------------- | ------------------------ | ----------------- |
| kratad     | host    | backend daemon   | ./hack/debug/kratad.sh   | crates/kratad     |
| kratanet   | host    | backend daemon   | ./hack/debug/kratanet.sh | crates/kratanet   |
| kratactl   | host    | CLI tool         | ./hack/debug/kratactl.sh | crates/kratactl   |
| krataguest | guest   | none, guest init | N/A                      | crates/krataguest |

You will find the code to each executable available in the bin/ and src/ directories inside
it's corresponding code path from the above table.

## Environment

| Component     | Specification | Notes                                                                             |
| ------------- | ------------- | --------------------------------------------------------------------------------- |
| Architecture  | x86_64        | aarch64 support requires minimal effort, but limited to x86 for research phase    |
| Memory        | At least 6GB  | dom0 will need to be configured will lower memory limit to give krata guests room | 
| Xen           | 4.17          | Temporary due to hardcoded interface version constants                            |
| Debian        | stable / sid  | Debian is recommended due to the ease of Xen setup                                |
| rustup        | any           | Install Rustup from https://rustup.rs                                             |

## Setup Guide

1. Install the specified Debian version on a x86_64 host _capable_ of KVM (NOTE: KVM is not used, Xen is a type-1 hypervisor).

2. Install required packages: `apt install git xen-system-amd64 flex bison libelf-dev libssl-dev bc`

3. Install [rustup](https://rustup.rs) for managing a Rust environment.

4. Configure `/etc/default/grub.d/xen.cfg` to give krata guests some room:

```sh
# Configure dom0_mem to be 4GB, but leave the rest of the RAM for krata guests.
GRUB_CMDLINE_XEN_DEFAULT="dom0_mem=4G,max:4G"
```

After changing the grub config, update grub: `update-grub`

Then reboot to boot the system as a Xen dom0.

You can validate that Xen is setup by running `xl info` and ensuring it returns useful information about the Xen hypervisor.

5. Clone the krata source code:
```sh
$ git clone https://github.com/edera-dev/krata.git krata
$ cd krata
```

6. Build a guest kernel image:

```sh
$ ./hack/kernel/build.sh -j4
```

7. Copy the guest kernel image at `target/kernel/kernel` to `/var/lib/krata/guest/kernel` to have it automatically detected by kratad.
8. Launch `./hack/debug/kratanet.sh` and keep it running in the foreground.
9. Launch `./hack/debug/kratad.sh` and keep it running in the foreground.
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
