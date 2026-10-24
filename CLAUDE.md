# embassy-knx

A KNX TP1 device stack for embedded boards, written in `no_std` Rust on [embassy](https://embassy.dev). An onsemi **NCN5130** transceiver handles the physical layer; this crate implements every layer above it, from the transceiver driver up to the application layer.

- **Target:** nRF5340 **network core** (`thumbv8m.main-none-eabihf`, 256 KB flash / 64 KB RAM, see `memory.x`).
- **Transceiver link:** UARTE `SERIAL0`, RX `P1.10`, TX `P1.11`, 38400 baud, even parity.
- **Two packages:** the repo root is the stack as a HAL-free `no_std` library (`embassy-knx`), which builds for the host. `firmware/` is the nRF5340 binary (`embassy-knx-firmware`), a separate package with its own `Cargo.lock`, `.cargo/config.toml` (ARM target, probe-rs runner), `memory.x` and `build.rs`.
- **Build:** `cd firmware && cargo build`. **Flash and run:** `cd firmware && cargo run`. Logging uses `defmt` over RTT; in the library, `defmt` is an optional feature that the firmware enables.
- **Tests:** `cargo test` at the repo root runs the host tests (unit tests, plus driver and end-to-end tests against a simulated NCN5130). Also run `cargo build --features defmt` and the firmware build after changes. See [`docs/testing.md`](docs/testing.md). `knxtest/` holds Node.js scripts (`knxultimate`) that talk to the device through a KNX/IP tunnel.

## Layout

| File | Role |
|---|---|
| `firmware/src/main.rs` | Startup, peripherals and `Uarte` adapter, frame pool and `Bus` statics, task spawning, placeholder user application loop |
| `src/lib.rs`, `src/fmt.rs` | Crate root; logging macros that forward to `defmt` or compile to nothing |
| `src/bus.rs` | `Bus`: the channels between driver and stack (`bus_rx`, `bus_tx`) and the L_Data.con signal |
| `src/ncn51_driver.rs` | NCN5130 UART driver, generic over `embedded_io_async`: frame RX/TX, ACK, L_Data.con |
| `src/frame.rs` | Frame pool, frame buffers and field accessors (`FrameReader`/`FrameWriter`) |
| `src/data_link_layer.rs`, `network_layer.rs`, `transport_layer.rs`, `application_layer.rs` | KNX layers 2–7, each owning the layer below it by value, all running in one task |
| `src/data_point.rs` | Datapoint type (DPT) encoding |
| `src/group_object*.rs`, `group_address_table.rs`, `interface_object_server.rs` | Stubs or drafts, not in the module tree |
| `src/test_util.rs`, `tests/` | Host test helpers (`StackHarness`, `Ncn5130Sim`) and integration tests |

## Constraints

- `no_std`, no heap. Use `heapless` and static allocation.
- Keep the library HAL-free: no `embassy-nrf` or `cortex-m` in the root package, and no global statics for state that tests need to isolate (pass it in, like `Bus`).
- Fix bugs test-first: write a host test that fails, then fix it.
- Prefer zero-copy: frames live in one pool buffer whose ownership moves between layers. Don't copy frame bytes between layers.
- Don't commit `specs/`. It holds the licensed KNX Standard PDFs, kept locally for reference only; the repo is public.

## Architecture review — read before changing the design

[`docs/architecture-review.md`](docs/architecture-review.md) holds the current architecture assessment:

- known bugs, with IDs B1–B9 and M1–M3;
- memory and zero-copy issues (Z1–Z6);
- the target task layout and the rules for each task boundary;
- open questions for the project owner;
- the recommended order of work.

Consult it before working on the driver, frames, layers, or task structure. When you resolve an item, update its **Status** in that file and mention its ID in the commit message. Don't guess answers to its open questions; ask.
