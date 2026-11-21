//! # wrapper-ble-esp32c3mini
//!
//! A `no_std` library wrapping the initialization of a BLE peripheral
//! (GATT peripheral role) on ESP32-C3, using:
//!
//! - [`esp-radio`](https://docs.rs/esp-radio) for the hardware HCI controller
//!   ([`BleConnector`]),
//! - [`bt-hci`](https://docs.rs/bt-hci) and its [`ExternalController`] to
//!   expose this controller in the format expected by the host stack,
//! - [`trouble-host`](https://docs.rs/trouble-host) for the BLE host stack
//!   (GAP, GATT, L2CAP) and asynchronous execution via Embassy.
//!
//! ## Overview
//!
//! The entry point is [`BleSystem::init`], which builds:
//! 1. the hardware BLE controller ([`BleController`]),
//! 2. the `trouble-host` host stack configured for the peripheral role,
//! 3. a [`BleServer`] GATT server exposing the [`DisplayService`] service.
//!
//! The returned [`Runner`](trouble_host::Runner) must then be driven
//! continuously by the [`run_ble_runner`] task, typically spawned on the
//! final binary's Embassy executor.
//!
//! ## Usage example (skeleton)
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use wrapper_ble_esp32c3mini::{BleSystem, run_ble_runner};
//!
//! #[esp_hal_embassy::main]
//! async fn main(spawner: embassy_executor::Spawner) {
//!     let peripherals = esp_hal::init(esp_hal::Config::default());
//!
//!     let BleSystem { runner, peripheral, server } = BleSystem::init(peripherals.BT);
//!
//!     spawner.spawn(ble_runner_task(runner)).unwrap();
//!
//!     // `peripheral` is used for advertising and accepting connections,
//!     // `server` exposes the `command` / `status` characteristics.
//! }
//!
//! #[embassy_executor::task]
//! async fn ble_runner_task(runner: trouble_host::Runner<'static, wrapper_ble_esp32c3mini::BleController, trouble_host::DefaultPacketPool>) {
//!     wrapper_ble_esp32c3mini::run_ble_runner(runner).await;
//! }
//! ```
//!
//! ## Version compatibility
//!
//! The versions specified in `Cargo.toml` are not arbitrary: they form a
//! compatible set of recent versions at the time this crate was written.
//!
//! - `esp-radio = "0.18.0"` requires `esp-hal` in the range `~1.1.0-rc.0`
//!   (so `esp-hal = "1.1.2"`, **not** the `1.2.x` branch) and `bt-hci`
//!   in the range `^0.8.0`.
//! - `bt-hci = "0.8.1"` is therefore selected, which in turn requires
//!   `trouble-host = "0.6.0"` (the branch of `trouble-host` that depends
//!   on `bt-hci ^0.8`; versions `0.7`/`0.8` moved to `bt-hci ^0.9`/`^0.10`).
//!
//! If `esp-radio` releases a version that follows `esp-hal 1.2.x` and a
//! newer `bt-hci`, `trouble-host` will need to be updated alongside
//! `bt-hci`, not separately.

#![no_std]

use bt_hci::controller::ExternalController;
use embassy_time::{Duration, Timer};
use esp_radio::ble::controller::BleConnector;
use static_cell::StaticCell;
use trouble_host::prelude::*;

/// Size in bytes of the HCI buffer used by the external controller.
///
/// `20` is the default HCI ACL payload size used in the `esp-radio` /
/// `trouble-host` examples. Increasing it allows negotiation of a larger
/// L2CAP MTU at the cost of more static RAM.
pub type BleController = ExternalController<BleConnector<'static>, 20>;

/// GATT server exposed by this BLE peripheral.
///
/// Contains a single [`DisplayService`] service, added during construction
/// via [`BleServer::new_with_config`].
#[gatt_server]
pub struct BleServer {
    /// GATT service exposing the `command` and `status` characteristics.
    pub display: DisplayService,
}

/// "Display" GATT service, based on Nordic UART Service (UUID `6e400001...`).
///
/// - `command`: writable characteristic used by the BLE client to send
///   commands to the device (up to 16 bytes).
/// - `status`: readable and notifiable characteristic used by the device
///   to report its status to the client (up to 16 bytes).
#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
pub struct DisplayService {
    /// Command sent by the client (write with or without response).
    #[characteristic(uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e", write, write_without_response)]
    pub command: heapless::Vec<u8, 16>,

    /// Current device status, readable and notifiable.
    #[characteristic(uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e", read, notify)]
    pub status: heapless::Vec<u8, 16>,
}

/// Groups the three components required to run the BLE stack: the host
/// stack runner, the peripheral role, and the application GATT server.
///
/// These three components are deliberately separate rather than kept in
/// a single opaque struct because `trouble-host` expects them to be used
/// independently: the `runner` runs in a background task
/// ([`run_ble_runner`]), while `peripheral` publishes BLE advertisements
/// and `server` responds to GATT reads and writes.
pub struct BleSystem {
    /// BLE host stack event loop; must run continuously.
    pub runner: Runner<'static, BleController, DefaultPacketPool>,
    /// Peripheral role: BLE advertising and accepting connections.
    pub peripheral: Peripheral<'static, BleController, DefaultPacketPool>,
    /// Application GATT server (see [`BleServer`]).
    ///
    /// `#[gatt_server]` generates a type with a lifetime parameter
    /// (`BleServer<'values>`) inherited from the string passed to
    /// `PeripheralConfig::name`. Since it is always passed a `&'static str`
    /// (the device name is a literal), the lifetime is always `'static` here.
    pub server: BleServer<'static>,
}

impl BleSystem {
    /// Initializes the hardware BLE controller, the `trouble-host` host
    /// stack, and the GATT server, returning all three components ready
    /// to use in a [`BleSystem`].
    ///
    /// # Static memory
    ///
    /// `trouble-host` requires its internal resources ([`HostResources`])
    /// and the [`Stack`] itself to live as long as the returned
    /// `runner`/`peripheral` (`'static`). They are given a `'static`
    /// lifetime using [`StaticCell`], the standard Embassy pattern for
    /// this situation.
    ///
    /// # Panics
    ///
    /// This function panics if:
    /// - [`BleConnector`] initialization fails (radio hardware unavailable
    ///   or misconfigured),
    /// - GATT server creation fails (for example, if the device name
    ///   exceeds the GAP maximum length of 22 bytes).
    ///
    /// Both failures are treated as unrecoverable at this stage of
    /// initialization and are reported by a panic rather than a `Result`.
    /// For recoverable error handling, replace the `.expect(...)` calls
    /// with propagation via `Result`.
    pub fn init(bt_peripheral: esp_hal::peripherals::BT<'static>) -> Self {
        let transport = BleConnector::new(bt_peripheral, esp_radio::ble::Config::default())
            .expect("Failed to initialize BleConnector");
        let controller: BleController = ExternalController::new(transport);

        // Internal host stack resources (L2CAP queues, etc.).
        // Generic parameters <PacketPool, CONNS, CHANNELS>: 1 simultaneous
        // connection, 1 L2CAP channel. Increase these constants to handle
        // multiple connections or channels in parallel.
        static RESOURCES: StaticCell<HostResources<DefaultPacketPool, 1, 1>> = StaticCell::new();
        let resources = RESOURCES.init(HostResources::new());

        // The `Stack` must also have a `'static` lifetime: `build()` expects
        // `&'stack self`, where `'stack` is the `Stack` lifetime parameter
        // inherited from `resources`.
        static STACK: StaticCell<Stack<'static, BleController, DefaultPacketPool>> =
            StaticCell::new();
        let stack = STACK.init(
            trouble_host::new(controller, resources)
                // Static random address (high bit set); replace with a
                // device-specific address in production (e.g. derived
                // from the unique hardware identifier).
                .set_random_address(Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff])),
        );

        let Host {
            peripheral,
            runner,
            ..
        } = stack.build();

        let server = BleServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: "ESP32C3-Mini",
            appearance: &appearance::UNKNOWN,
        }))
        .expect("Failed to create GATT server");

        Self {
            runner,
            peripheral,
            server,
        }
    }
}

/// Runs the BLE host stack event loop indefinitely.
///
/// Spawn this on the final binary's Embassy executor (see the module
/// example). If the runner encounters an error (such as an unexpected
/// controller disconnection), the error is logged through the [`log`]
/// facade and the loop retries after one second rather than panicking,
/// so the rest of the application can continue.
///
/// The final binary must initialize a `log` backend (for example,
/// `esp-println` with its `log-04` feature, or `defmt`) for these messages
/// to be visible; without an initialized backend, they are silently ignored
/// rather than causing an error.
pub async fn run_ble_runner(mut runner: Runner<'_, BleController, DefaultPacketPool>) {
    loop {
        if let Err(e) = runner.run().await {
            log::error!("Bluetooth runner error: {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}