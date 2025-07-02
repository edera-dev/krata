# Edera Technical Overview

## What is Edera?

Edera is a secure-by-default, cloud-native platform built on a reimagined, memory-safe type-1 hypervisor. It unlocks hard multitenancy and strong container isolation—without the performance hit.

Unlike traditional container runtimes that share a single Linux kernel across containers, Edera runs each container in a lightweight virtual machine (called a **zone**), with its own dedicated Linux kernel. This eliminates the kernel as a shared attack surface.

And because Edera doesn’t rely on nested virtualization, it runs wherever containers do—across public clouds, on-prem, and edge environments.

## How Edera Works

At its core, Edera uses a custom hypervisor based on Xen, with key components rewritten in Rust for safety, performance, and maintainability. Edera introduces the concept of **zones**—independent, fast-booting virtual machines that serve as security boundaries for container workloads.

Each zone runs its own Linux kernel and minimal init system. The kernel and other system components are delivered via OCI images, keeping things composable, cacheable, and consistent.

Zones are paravirtualized using the Xen PV protocol. This keeps them lightweight and fast—no hardware virtualization required. But when hardware support is available (e.g., on x86 with VT-x), Edera uses it to get near bare-metal performance.

## How Edera Runs & Secures Containers

Edera allows you to compose your infrastructure the same way you compose workloads: using OCI images.

Each zone consumes a small number of OCI images:
- A **kernel image** that provides the zone kernel.
- One or more **system extension images** that provide init systems, utilities, and kernel modules.
- Optionally, **driver zones**—zones that provide shared services (like networking) to other zones.

Inside each zone, container workloads run via a minimal OCI runtime called **Styrolite**, written in Rust. Unlike traditional setups (like Kata Containers, which layer containerd and runc as external processes), Styrolite is embedded inside the zone itself.

### Key Benefits of This Design
- No external container runtime processes  
- Zone init system directly manages containers  
- Minimal attack surface, optimized for secure execution

This tightly integrated design avoids the complexity, latency, and exposure introduced by conventional container runtimes. It keeps the execution path short, verifiable, and secure-by-design.

## Zones as Security Boundaries

In Kubernetes, Edera runs pods inside **zones**—isolated virtual machines that eliminate risks like container escape, privilege escalation, and lateral movement.

Each zone boots its own kernel, pulled via OCI, and runs a single pod by default. You can also configure zones to run a replica set, a namespace, or a set of trusted workloads together.

To use Edera, apply the `RuntimeClass`:

```yaml
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: edera
handler: edera
```

Then annotate your pod:

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: edera-protect-pod
spec:
  runtimeClassName: edera
```

This causes the pod to be scheduled to a node running Edera’s hypervisor. The pod is transparently launched inside its own VM zone—no image changes, no config rewrites, and no extra work from developers.

## What Exactly Is an Edera Zone?

An Edera zone is a minimal VM built from OCI-delivered components. At launch time, the Edera daemon unpacks:

### Kernel Image
Located under `/kernel` in the OCI image:
- `image`: the Linux kernel (vmlinuz)
- `metadata`: key-value pairs for boot parameters
- `addons.squashfs`: includes kernel modules in `/modules`
- `config.gz`: the kernel configuration file

### Initramfs Contents
Packaged in a CPIO archive, typically mounted from:
`usr/lib/edera/protect/zone/initrd`

The initramfs includes:
- `/init`: static Rust binary that initializes the zone
- `/bin/styrolite`: embedded container runtime
- `/bin/zone`: control plane for managing containers and services via IDM (inter-domain messaging)

This structure lets Edera launch zones rapidly, with well-defined boundaries and no dependency on the host OS kernel. Everything the workload touches is defined, versioned, and validated.
