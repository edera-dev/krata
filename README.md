# krata

The Edera Hypervisor

![license](https://img.shields.io/github/license/edera-dev/krata)
![discord](https://img.shields.io/discord/1207447453083766814?label=discord)
[![check](https://github.com/edera-dev/krata/actions/workflows/check.yml/badge.svg)](https://github.com/edera-dev/krata/actions/workflows/check.yml)
[![nightly](https://github.com/edera-dev/krata/actions/workflows/nightly.yml/badge.svg)](https://github.com/edera-dev/krata/actions/workflows/nightly.yml)

---

- [Frequently Asked Questions](FAQ.md)
- [Development Guide](DEV.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

## Introduction

krata is a single-host hypervisor service built for OCI-compliant containers. It isolates containers using a type-1 hypervisor, providing workload isolation that can exceed the security level of KVM-based OCI-compliant runtimes.

krata utilizes the core of the Xen hypervisor, with a fully memory-safe Rust control plane to bring Xen tooling into a new secure era.

## Hardware Support

| Architecture | Completion Level | Virtualization Technology |
| ------------ | ---------------- | ------------------------- |
| x86_64       | 100% Completed   | Intel VT-x, AMD-V         |
| aarch64      | 30% Completed    | AArch64 virtualization    |
