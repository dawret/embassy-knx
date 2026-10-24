# Testing

The stack is tested in two tiers. Host tests run on the development machine with `cargo test` and need no hardware. MCU tests (planned, not yet written) run on the nRF5340 with the NCN5130 and a real KNX bus.

## Repository layout

| Path | Builds for | Contents |
|---|---|---|
| `/` (`embassy-knx`) | any target | The stack as a HAL-free `no_std` library. `defmt` is an optional feature; without it, the logging macros in `src/fmt.rs` compile to nothing. |
| `firmware/` (`embassy-knx-firmware`) | `thumbv8m.main-none-eabihf` | The nRF5340 network-core binary: peripheral setup, frame pool, `Bus`, tasks. Has its own `.cargo/config.toml` (target, probe-rs runner), `memory.x` and `build.rs`. |

What makes the library testable on the host:

- `NCN51Driver<U, L>` takes any `embedded_io_async::{Read, Write}` as the UART and any `embedded_hal::digital::StatefulOutputPin` as the RX LED. The firmware passes embassy-nrf's `BufferedUarte`, which receives continuously into a ring buffer, so `read` returns whatever has arrived and dropping a read loses nothing.
- The queues between the driver and the stack live in a `Bus` struct (`src/bus.rs`) that is passed in, not in global statics. Each test gets its own `Bus`, so tests run in parallel.
- All embassy-sync primitives use `CriticalSectionRawMutex`. On the host, `critical-section`'s `std` implementation provides it; it is enabled as a dev-dependency.
- The frame pool lives in `frame.rs` and is filled by `init_frame_pool()`. heapless 0.9 needs its `portable-atomic` feature for the pool on 64-bit targets; `Cargo.toml` enables it only there.

## Host tests

```sh
cargo test                       # at the repo root: all host tests
cargo test --lib transport       # one module
cargo test --test driver         # one integration test file
cargo build --features defmt     # check the defmt code paths still compile
```

To build and flash the firmware:

```sh
cd firmware
cargo build --release
cargo run --release              # flash and run via probe-rs
```

Async code runs on `futures-executor`'s `block_on`, with embassy-time's `std` time driver (`generic-queue-64`). Every test body runs under a 2 s timeout, so a stack that hangs fails the test instead of blocking the run.

### Unit tests (`src/**`, `#[cfg(test)]`)

| Module | Covers |
|---|---|
| `frame.rs` | Parsing and building standard and extended frames, addresses, field setters, length limits, frames from datapoints. |
| `data_point.rs` | DPT lengths and encoding. |
| `transport_layer.rs` | TPDU classification and the connection state machine (connect, ACK, repeats, sequence-number wrap, rejecting a second connection, disconnect). |

`src/test_util.rs` holds the shared helpers. `StackHarness` builds the real data link, network and transport layers on a private `Bus` and stands in for the driver: `inject()` delivers a received telegram, and every frame the stack sends is recorded (`sent()`) and confirmed with L_Data.con(Ok).

### Integration tests (`tests/`)

`tests/common/mod.rs` contains `Ncn5130Sim`, a simulated NCN5130. The driver talks to it through `SimUart`, two `embassy_sync::pipe::Pipe`s that implement `embedded_io_async`. The test drives the bus side:

- `bus_receive(telegram)` writes the telegram and its checksum to the UART in one go, then waits for bus idle time (`bus_idle()`). The driver uses a 2.6 ms silence to detect the end of a frame, so two telegrams without a gap would merge, exactly as on real hardware.
- `expect_ack()` checks that the driver sent U_AckInformation.
- `host_writes()` counts the driver's UART `write` calls.
- `transmit(con_ok)` serves one L_Data request: it decodes the U_L_DataStart/Continue/End command stream, echoes the telegram and answers with a positive or negative L_Data.con. It returns the telegram, including the checksum the driver computed.

| File | Covers |
|---|---|
| `tests/driver.rs` | The driver at the byte level: receiving, ACK only for our own individual address and group frames, checksum errors, a full-length extended frame, stray bytes, transmission with positive and negative confirmation, a standard frame sent in one UART write. |
| `tests/stack.rs` | End to end, with the real driver, the full stack and the test acting as the application: a GroupValueRead answered with a GroupValueResponse, and a T_Data_Connected answered with a T_ACK. |

### Writing new tests

- Write telegrams as hex octets without the checksum, and use `with_checksum()` where the complete telegram is needed.
- Frames the stack builds end in an empty checksum slot that the driver fills in. `StackHarness::sent()` strips it.

## MCU tests (planned)

Two kinds, both under `firmware/`:

1. **On-target tests** in `firmware/tests/`, using the [`embedded-test`](https://crates.io/crates/embedded-test) crate. `cargo test` in `firmware/` flashes each test binary through probe-rs and reports the results on the host. They cover what the host cannot: the real `Uarte` and NCN5130 (reset and U_State once implemented), pool and channel sizing, and timing, such as whether the ACK is sent within the NCN5130's window.
2. **Hardware-in-the-loop system tests**: the board on a real TP bus together with a KNX/IP interface. A host-side runner, an extension of the existing `knxtest/` scripts (`knxultimate`), sends telegrams through the tunnel and checks the device's replies while the board runs a test firmware image.

The scenarios in `tests/stack.rs` are deliberately written as telegram in, telegram out, so they can be ported to the hardware-in-the-loop runner.
