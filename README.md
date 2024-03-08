# krata

The Edera Hypervisor

![license](https://img.shields.io/github/license/edera-dev/krata)
![discord](https://img.shields.io/discord/1207447453083766814?label=discord)
[![check](https://github.com/edera-dev/krata/actions/workflows/check.yml/badge.svg)](https://github.com/edera-dev/krata/actions/workflows/check.yml)
[![nightly](https://github.com/edera-dev/krata/actions/workflows/nightly.yml/badge.svg)](https://github.com/edera-dev/krata/actions/workflows/nightly.yml)

---

- [Frequently Asked Questions](FAQ.md)
- [Development Guide](DEV.md)
- [Licensing Guide](LICENSING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

## Introduction

The krata hypervisor makes it possible to launch OCI containers on a Xen hypervisor without utilizing the Xen userspace tooling. krata contains just enough of the userspace of Xen (reimplemented in Rust) to start an x86_64 Xen Linux PV guest, and implements a Linux init process that can boot an OCI container. It does so by converting an OCI image into a squashfs file and packaging basic startup data in a bundle which the init container can read.

In addition, due to the desire to reduce dependence on the dom0 network, krata contains a networking daemon called kratanet. kratanet listens for krata guests to startup and launches a userspace networking environment. krata guests can access the dom0 networking stack via the proxynat layer that makes it possible to communicate over UDP, TCP, and ICMP (echo only) to the outside world. In addition, each krata guest is provided a "gateway" IP (both in IPv4 and IPv6) which utilizes smoltcp to provide a virtual host. That virtual host in the future could dial connections into the container to access container networking resources.

krata is in its early days and this project is still a work in progress.
