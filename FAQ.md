# Frequently Asked Questions

## How does krata currently work?

The krata hypervisor makes it possible to launch OCI containers on a Xen hypervisor without utilizing the Xen userspace tooling. krata contains just enough of the userspace of Xen (reimplemented in Rust) to start an x86_64 Xen Linux PV guest, and implements a Linux init process that can boot an OCI container. It does so by converting an OCI image into a squashfs/erofs file and packaging basic startup data in a bundle which the init container can read.

In addition, due to the desire to reduce dependence on the dom0 network, krata contains a networking daemon called kratanet. kratanet listens for krata guests to startup and launches a userspace networking environment. krata guests can access the dom0 networking stack via the proxynat layer that makes it possible to communicate over UDP, TCP, and ICMP (echo only) to the outside world. In addition, each krata guest is provided a "gateway" IP (both in IPv4 and IPv6) which utilizes smoltcp to provide a virtual host. That virtual host in the future could dial connections into the container to access container networking resources.

## Why utilize Xen instead of KVM?

Xen is a very interesting technology, and Edera believes that type-1 hypervisors are ideal for security. Most OCI isolation techniques use KVM, which is not a type-1 hypervisor, and thus is subject to the security limitations of the OS kernel. A type-1 hypervisor on the otherhand provides a minimal amount of attack surface upon which less-trusted guests can be launched on top of.

## Why not utilize pvcalls to provide access to the host network?

pvcalls is extremely interesting, and although it is certainly possible to utilize pvcalls to get the job done, we chose to utilize userspace networking technology in order to enhance security. Our goal is to drop the use of all xen networking backend drivers within the kernel and have the guest talk directly to a userspace daemon, bypassing the vif (xen-netback) driver. Currently, in order to develop the networking layer, we utilize xen-netback and then use raw sockets to provide the userspace networking layer on the host.

## What are the future plans?

Edera is building a company to compete in the hypervisor space with open-source technology. More information to come soon on official channels.
