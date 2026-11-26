# wrapper-ble-esp32c3mini

[![Crates.io](https://img.shields.io/crates/v/wrapper-ble-esp32c3mini.svg)](https://crates.io/crates/wrapper-ble-esp32c3mini)
[![docs.rs](https://img.shields.io/docsrs/wrapper-ble-esp32c3mini)](https://docs.rs/wrapper-ble-esp32c3mini)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20or%20Apache--2.0-blue.svg)](https://opensource.org/licenses/MIT)
[![Downloads](https://img.shields.io/crates/d/wrapper-ble-esp32c3mini.svg)](https://crates.io/crates/wrapper-ble-esp32c3mini)


A `no_std` library that wraps the initialization of a BLE peripheral
(GATT peripheral) on ESP32-C3, built on `esp-radio`, `bt-hci`, and
`trouble-host`.

It provides:

- `BleController`: an alias for the external HCI controller connected to
  `esp-radio`'s `BleConnector`;
- `BleServer` / `DisplayService`: a minimal GATT server with two
  characteristics (`command` for writing, `status` for reading/notification);
- `BleSystem::init(...)`: builds the controller, the `trouble-host` host
  stack, and the GATT server, returning ready-to-use `runner`, `peripheral`,
  and `server` components;
- `run_ble_runner(...)`: a task to spawn to continuously run the host
  stack's event loop.

## Installation

```toml
[dependencies]
wrapper-ble-esp32c3mini = "1.1"
```

## Version history

- **1.0.0** stable release; documentation and messages translated into English.
- **0.2.0** removed the direct dependency on `esp-println` (replaced by
  the `log` facade; see Logging below). Version 0.1.0 could break the build
  of a consuming binary that selected a different version of `esp-println`,
  because it declares `links = "esp-println"` (only one version is allowed
  in the entire dependency tree).
- **0.1.0** initial release.

## Why these dependency versions?

`Cargo.toml` deliberately specifies certain versions rather than selecting
the latest release of each crate, because they are **not** all compatible
with one another at the time of writing:

| Crate          | Selected version | Reason                                                                 |
|----------------|------------------|-------------------------------------------------------------------------|
| `esp-radio`    | `0.18.0`         | latest stable release                                                   |
| `esp-hal`      | `1.1.2`          | `esp-radio 0.18.0` requires `esp-hal ~1.1.0-rc.0`: the `1.1.x` branch only, not `1.2.x` |
| `bt-hci`       | `0.8.1`          | `esp-radio 0.18.0` requires `bt-hci ^0.8.0`                             |
| `trouble-host` | `0.6.0`          | the branch of `trouble-host` that depends on `bt-hci ^0.8` (versions `0.7`/`0.8` moved to `bt-hci ^0.9`/`^0.10`) |
| `heapless`     | `0.9`            | required by `trouble-host 0.6.0`                                        |
| `embassy-sync` | `0.7.2`          | direct dependency required: `#[gatt_server]`/`#[gatt_service]` expansion by `trouble-host-macros 0.4.0` (the version resolved by `trouble-host 0.6.0`, which requires `^0.4.0`) references `embassy_sync::...` without a `trouble_host` re-export |



## Logging

`run_ble_runner` logs errors through the [`log`](https://docs.rs/log)
facade rather than using `esp-println` directly. `esp-println` declares
`links = "esp-println"` (only one version can appear in the entire dependency
tree) **and** requires exactly one output feature (`auto`, `uart`,
`jtag-serial`, or `no-op`). A library should not make these choices on
behalf of the final binary, as they could conflict with its own version or
feature selection. The final binary remains free to use `esp-println`
(with its `log-04` feature), `defmt`, or another `log` backend.

## Static memory

`trouble-host` requires its internal resources (`HostResources`) and the
`Stack` itself to have a `'static` lifetime. `BleSystem::init` therefore uses
`static_cell::StaticCell` to give them that lifetime rather than keeping them
in local variables, which would not satisfy the lifetime bounds expected by
`Runner<'static, ...>` / `Peripheral<'static, ...>`.

## Lifetimes (`'static`)

Two other parts of the signature are deliberately set to `'static`:

- **`BleServer<'static>`**: `#[gatt_server]` generates a type with a
  lifetime parameter (`BleServer<'values>`) inherited from the string
  passed to `PeripheralConfig::name`. Since it is always passed a
  `&'static str` (the device name is a literal), the lifetime is always
  `'static` here.
- **`bt_peripheral: esp_hal::peripherals::BT<'static>`**:
  `esp_hal::peripherals::BT` also has a lifetime parameter. Without an
  explicit annotation in the signature of `BleSystem::init`, the compiler
  assigns it an anonymous lifetime specific to the call, incompatible with
  `BleController = ExternalController<BleConnector<'static>, 20>`.
  `Peripherals` normally obtained through `esp_hal::init(...)` has
  `'static` fields by default, so `peripherals.BT` can be used directly.

## Minimal example

```rust,ignore
#![no_std]
#![no_main]

use wrapper_ble_esp32c3mini::{BleSystem, run_ble_runner};

#[esp_hal_embassy::main]
async fn main(spawner: embassy_executor::Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let ble = BleSystem::init(peripherals.BT);
    spawner.spawn(ble_runner_task(ble.runner)).unwrap();

    // `ble.peripheral`: BLE advertising and accepting connections
    // `ble.server`: reading/writing GATT characteristics
}

#[embassy_executor::task]
async fn ble_runner_task(
    runner: trouble_host::Runner<'static, wrapper_ble_esp32c3mini::BleController, trouble_host::DefaultPacketPool>,
) {
    run_ble_runner(runner).await;
}
```

The final binary is responsible for configuring the Embassy executor (via
`esp-hal-embassy`); this library only provides the futures and GATT structure.

**Build, documentation, and publishing**
```bash
cargo build --target riscv32imc-unknown-none-elf --release
cargo doc --target riscv32imc-unknown-none-elf --release
cargo publish --target riscv32imc-unknown-none-elf 
```

## 🛡️ License

Licensed under either of:

    MIT license (http://opensource.org/licenses/MIT)

    Apache License, Version 2.0 (http://www.apache.org/licenses/LICENSE-2.0)

at your option.

## 🦅 About

Developed by Jorge Andre Castro.
