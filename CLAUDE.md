# embassy-knx

A KNX TP1 device stack for embedded boards, written in `no_std` Rust on [embassy](https://embassy.dev). An onsemi **NCN5130** transceiver handles the physical layer; this crate implements every layer above it, from the transceiver driver up to the application layer.

- **Target:** nRF5340 **network core** (`thumbv8m.main-none-eabihf`, 256 KB flash / 64 KB RAM, see `memory.x`).
- **Transceiver link:** UARTE `SERIAL0`, RX `P1.10`, TX `P1.11`, 38400 baud, even parity.
- **Build:** `cargo build`. **Flash and run:** `cargo run`, which uses the probe-rs runner configured in `.cargo/config.toml`. Logging uses `defmt` over RTT.
- **Tests:** there are no host tests yet. `knxtest/` holds Node.js scripts (`knxultimate`) that talk to the device through a KNX/IP tunnel.

## Layout

| File | Role |
|---|---|
| `src/main.rs` | Startup, resource assignment, task spawning, placeholder user application loop |
| `src/ncn51_driver.rs` | NCN5130 UART driver: frame RX/TX, ACK, L_Data.con. Owns the frame pool and bus channels. |
| `src/frame.rs` | Frame buffers (pool-allocated) and field accessors (`FrameReader`/`FrameWriter`) |
| `src/data_link_layer.rs`, `network_layer.rs`, `transport_layer.rs`, `application_layer.rs` | KNX layers 2–7, each owning the layer below it by value, all running in one task |
| `src/data_point.rs` | Datapoint type (DPT) encoding |
| `src/group_object*.rs`, `group_address_table.rs`, `interface_object_server.rs` | Stubs or drafts, not yet functional |

## Constraints

- `no_std`, no heap. Use `heapless` and static allocation.
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
