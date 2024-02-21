# krata

An early version of the Edera hypervisor. Not for production use.

[Join our community Discord](https://discord.gg/UGZCtX9NG9), or follow the founders [Alex](https://social.treehouse.systems/@alex) and [Ariadne](https://social.treehouse.systems/@ariadne) on Mastodon to follow the future of krata.

## What is krata?

The krata prototype makes it possible to launch OCI containers on a Xen hypervisor without utilizing the Xen userspace tooling. krata contains just enough of the userspace of Xen (reimplemented in Rust) to start an x86_64 Xen Linux PV guest, and implements a Linux init process that can boot an OCI container. It does so by converting an OCI image into a squashfs file and packaging basic startup data in a bundle which the init container can read.

In addition, due to the desire to reduce dependence on the dom0 network, krata contains a networking daemon called kratanet. kratanet listens for krata guests to startup and launches a userspace networking environment. krata guests can access the dom0 networking stack via the proxynat layer that makes it possible to communicate over UDP, TCP, and ICMP (echo only) to the outside world. In addition, each krata guest is provided a "gateway" IP (both in IPv4 and IPv6) which utilizes smoltcp to provide a virtual host. That virtual host in the future could dial connections into the container to access container networking resources.

krata is in it's early days and this project is provided as essentially a demo of what an OCI layer on Xen could look like.

## FAQs

### Why utilize Xen instead of KVM?

Xen is a very interesting technology, and Edera believes that type-1 hypervisors are ideal for security. Most OCI isolation techniques use KVM, which is not a type-1 hypervisor, and thus is subject to the security limitations of the OS kernel. A type-1 hypervisor on the otherhand provides a minimal amount of attack surface upon which less-trusted guests can be launched on top of.

### Why not utilize pvcalls to provide access to the host network?

pvcalls is extremely interesting, and although it is certainly possible to utilize pvcalls to get the job done, we chose to utilize userspace networking technology in order to enhance security. Our goal is to drop the use of all xen networking backend drivers within the kernel and have the guest talk directly to a userspace daemon, bypassing the vif (xen-netback) driver. Currently, in order to develop the networking layer, we utilize xen-netback and then use raw sockets to provide the userspace networking layer on the host.

### Why is this prototype utilizing AGPL?

This repository is licensed under AGPL. This is because what is here is not intended for anything other than curiosity and research. Edera will utilize a different license for any production versions of krata.

As such, no external contributions are accepted at this time.

### Are external contributions accepted?

Currently no external contributions are accepted. krata is in it's early days and the project is provided under AGPL. Edera may decide to change licensing as we start to build future plans, and so all code here is provided to show what is possible, not to work towards any future product goals.

### What are the future plans?

Edera is building a company to compete in the hypervisor space with open-source technology. More information to come soon on official channels.

## Development Guide

### Structure

krata is composed of three major executables:

| Executable | Runs On | User Interaction | Dev Runner                  | Code Path   |
| ---------- | ------- | ---------------- | --------------------------- | ----------- |
| kratanet   | host    | backend daemon   | ./scripts/kratanet-debug.sh | network     |
| kratactl   | host    | CLI tool         | ./scripts/kratactl-debug.sh | controller  |
| kratactr   | guest   | none, guest init | N/A                         | container   |

You will find the code to each executable available in the bin/ and src/ directories inside
it's corresponding code path from the above table.

### Environment

| Component     | Specification | Notes                                                                             |
| ------------- | ------------- | --------------------------------------------------------------------------------- |
| Architecture  | x86_64        | aarch64 support requires minimal effort, but limited to x86 for research phase    |
| Memory        | At least 6GB  | dom0 will need to be configured will lower memory limit to give krata guests room | 
| Xen           | 4.17          | Temporary due to hardcoded interface version constants                            |
| Debian        | stable / sid  | Debian is recommended due to the ease of Xen setup                                |
| rustup        | any           | Install Rustup from https://rustup.rs                                             |

### Debian Setup

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
$ ./kernel/build.sh -j4
```

7. Copy the guest kernel image at `kernel/target/kernel` to `/var/lib/krata/default/kernel` to have it automatically detected by kratactl.
8. Launch `./scripts/kratanet-debug.sh` and keep it running in the foreground.
9. Run kratactl to launch a container:

```sh
$ ./scripts/kratactl-debug.sh launch --attach mirror.gcr.io/library/alpine:latest /bin/busybox sh
```

To detach from the container console, use `Ctrl + ]` on your keyboard.

To list the running containers, run:
```sh
$ ./scripts/kratactl-debug.sh list
```

To destroy a running container, copy it's UUID from either the launch command or the container list and run:
```sh
$ ./scripts/kratactl-debug.sh destroy CONTAINER_UUID
```
