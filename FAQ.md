# Frequently Asked Questions

## Why utilize Xen instead of KVM?

Xen is a very interesting technology, and Edera believes that type-1 hypervisors are ideal for security. Most OCI isolation techniques use KVM, which is not a type-1 hypervisor, and thus is subject to the security limitations of the OS kernel. A type-1 hypervisor on the other hand provides a minimal attack surface upon which less-trusted guests can be launched on top of.

## Why not utilize pvcalls to provide access to the host network?

pvcalls is fascinating, and although it is certainly possible to utilize pvcalls to get the job done, we chose to utilize userspace networking technology in order to enhance security. Our goal is to drop the use of all xen networking backend drivers within the kernel and have the guest talk directly to a userspace daemon, bypassing the vif (xen-netback) driver. Currently, in order to develop the networking layer, we utilize xen-netback and then use raw sockets to provide the userspace networking layer on the host.
