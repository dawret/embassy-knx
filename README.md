# embassy-knx

A KNX TP1 device stack for embedded boards, written in `no_std` Rust on [embassy](https://embassy.dev). An onsemi **NCN5130** transceiver handles the physical layer; this crate implements every layer above it, from the transceiver driver up to the application layer.

The stack is work in progress: group communication works, while connection-oriented communication and ETS programming are incomplete. See [`docs/architecture-review.md`](docs/architecture-review.md) for the current state and known issues.

## Repository layout

| Path | What it is | Builds for |
|---|---|---|
| `/` (`embassy-knx`) | The stack as a hardware-independent `no_std` library, with host tests in `src/` and `tests/` | the host (x86_64) or ARM |
| `firmware/` (`embassy-knx-firmware`) | The firmware for the nRF5340 **network core**: peripheral setup and tasks | ARM only (`thumbv8m.main-none-eabihf`) |
| `knxtest/` | Node.js scripts that talk to the device over a KNX/IP tunnel | — |
| `docs/` | Architecture review and testing guide | — |

`firmware/` is a separate Cargo package with its own `.cargo/config.toml`, which selects the ARM target and the probe-rs runner. Run firmware commands from inside `firmware/`, and library and test commands from the repo root.

## Hardware

- nRF5340 (256 KB flash / 64 KB RAM on the network core, see `firmware/memory.x`)
- NCN5130 KNX transceiver on UARTE `SERIAL0`: RX `P1.10`, TX `P1.11`, 38400 baud, even parity
- RX activity LED on `P1.07`, status LED on `P1.05`
- A debug probe supported by [probe-rs](https://probe.rs) (e.g. the J-Link on an nRF5340 DK)

The device's individual address is currently fixed at compile time to `1.1.120` (`src/settings.rs`).

## Prerequisites

- Rust via [rustup](https://rustup.rs) (stable toolchain)
- A C toolchain for the host linker: Rust needs it to link build scripts and the host test binaries.
  - Fedora: `sudo dnf install gcc`
  - Debian/Ubuntu: `sudo apt install build-essential`
- For the firmware, the ARM target:
  ```sh
  rustup target add thumbv8m.main-none-eabihf
  ```
- For flashing, probe-rs. Either follow the [installation guide](https://probe.rs/docs/getting-started/installation/), or run:
  ```sh
  cargo install probe-rs-tools --locked
  ```
- The pre-commit hook, which runs `cargo fmt --check` and `cargo clippy -D warnings` on both packages:
  ```sh
  git config core.hooksPath .githooks
  ```
  It checks the working tree, so unstaged changes count. `git commit --no-verify` skips it.

## Build

### Library (x86)

From the repo root:

```sh
cargo build                      # library for the host, logging compiled out
cargo build --features defmt     # with defmt logging, as the firmware uses it
```

### Firmware (ARM)

From `firmware/`:

```sh
cd firmware
cargo build                      # debug build
cargo build --release            # release build
```

The binaries end up in `firmware/target/thumbv8m.main-none-eabihf/{debug,release}/embassy-knx-firmware`.

## Flash and run

Connect the probe, then from `firmware/`:

```sh
cargo run --release
```

This builds the firmware and runs it via `probe-rs run --chip nRF5340_xxAA`, which flashes the ELF and streams the `defmt` log over RTT to the terminal. Press `Ctrl+C` to stop.

The log level comes from `DEFMT_LOG` in `firmware/.cargo/config.toml` (default `info`). Override it for a single run like this:

```sh
DEFMT_LOG=debug cargo run --release
```

### Debugging in VS Code

`.vscode/launch.json` has a `probe-rs-debug` configuration ("Launch") that flashes the release binary onto core 1 (the network core). It does not build the firmware first, so run `cargo build --release` in `firmware/` beforehand. It needs the [probe-rs VS Code extension](https://probe.rs/docs/tools/debugger/).

## Tests

### Host tests

From the repo root:

```sh
cargo test                       # all host tests
cargo test --lib                 # unit tests only
cargo test --lib transport       # tests whose name contains "transport"
cargo test --test driver         # one integration test file (tests/driver.rs)
cargo test --test stack          # end-to-end tests (tests/stack.rs)
```

The host tests need no hardware:

- **Unit tests** cover frame parsing and building, datapoint encoding and the transport-layer connection state machine.
- **Integration tests** run the real NCN5130 driver and the full stack against a simulated transceiver, at the UART byte level.

See [`docs/testing.md`](docs/testing.md) for how the tests are structured and how to write new ones.

Before submitting a change, also check that the `defmt` code paths and the firmware still compile. The host tests don't build either of them.

```sh
cargo build --features defmt
(cd firmware && cargo build --release)
```

### Tests on the device

There are no automated on-device tests yet; they are planned in [`docs/testing.md`](docs/testing.md#mcu-tests-planned). In the meantime, `knxtest/` contains Node.js scripts for manual checks over a KNX/IP interface on the same bus:

```sh
cd knxtest
npm install
node index_ip.js                 # reads group address 1/1/98 via the knx-ip package
node index.js                    # connection test with knxultimate
```

The KNX/IP interface's address is hardcoded in the scripts (`10.0.20.185:3671`); adjust it for your setup.
