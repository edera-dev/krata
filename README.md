# krata

An isolation engine for securing compute workloads.

```bash
$ kratactl zone launch -a alpine:latest
```

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

krata is a single-host workload isolation service. It isolates workloads using a type-1 hypervisor, providing a tight security boundary while preserving performance.

krata utilizes the core of the Xen hypervisor with a fully memory-safe Rust control plane.

## Hardware Support

| Architecture | Completion Level | Hardware Virtualization |
|--------------|------------------|-------------------------|
| x86_64       | 100% Completed   | None, Intel VT-x, AMD-V |
| aarch64      | 10% Completed    | AArch64 virtualization  |
