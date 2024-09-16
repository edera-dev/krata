# krata

krata is an implementation of a Xen control-plane in Rust.

![license](https://img.shields.io/github/license/edera-dev/krata)
![discord](https://img.shields.io/discord/1207447453083766814?label=discord)
[![check](https://github.com/edera-dev/krata/actions/workflows/check.yml/badge.svg)](https://github.com/edera-dev/krata/actions/workflows/check.yml)

---

- [Frequently Asked Questions](FAQ.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

## Introduction

krata is a component of [Edera Protect](https://edera.dev/protect-kubernetes), for secure-by-design infrastructure.
It provides the base layer upon which Edera Protect zones are built on: a securely booted virtualization guest on the Xen hypervisor.

## Hardware Support

| Architecture | Completion Level | Hardware Virtualization |
|--------------|------------------|-------------------------|
| x86_64       | 100% Completed   | None, Intel VT-x, AMD-V |
| aarch64      | 10% Completed    | AArch64 virtualization  |
