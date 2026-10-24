# Architecture Review

- **Date:** 2026-10-24
- **Reviewed commit:** `7c41143` ("snapshot")
- **Method:** static review of all source files. The code was not built or run during the review, so no measurements were taken.
- **Status updates:** 2026-10-24, host test suite added (`docs/testing.md`). B10, B11 and M4 were found afterwards.
- **Scope:** architecture, memory/zero-copy, task (thread) boundaries, correctness.

Line numbers refer to the reviewed commit and will drift. Search for the quoted code if they no longer match. When an item is resolved, update its **Status** here rather than deleting it.

---

## 1. Current state

| Layer | File | State |
|---|---|---|
| NCN5130 driver | `src/ncn51_driver.rs` | Receives and sends L_Data with a basic ACK. No reset/init sequence and no U_State handling (`CtrlMsg` is empty). |
| Data link | `src/data_link_layer.rs` | Pass-through. The L_Data.con status is discarded and repeated frames are not de-duplicated. |
| Network | `src/network_layer.rs` | Pass-through: classifies by destination address. Hop count is not handled. |
| Transport | `src/transport_layer.rs` | Group and broadcast work. The connection-oriented state machine is partly written and panics at runtime (bug B1). |
| Application | `src/application_layer.rs` | GroupValueRead only. The ASAP is hardcoded to 0 and the group address to 1/1/98 (`transport_layer.rs:72`). |
| Group objects | `src/group_object.rs` | Unworkable draft: it takes `&'static mut self` on a non-`mut` static. |
| Tables | `src/group_*_table.rs` | Empty files. |
| Interface Objects / errors | `src/interface_object_server.rs`, `src/error.rs` | Not in the module tree and would not compile (`#[reprt]` typo, bad `use`). |

### Keep these design decisions

- **Zero-copy frame ownership.** A received frame lives in one buffer allocated from a pool (`heapless` `box_pool`). Ownership of that buffer moves through the layers, and each layer reads its fields through accessors at fixed offsets (`FrameReader`/`FrameWriter`). A `Frame` is about 8 bytes (pointer plus tag), so channels and layer enums never copy the frame bytes. This suits KNX TP, where the TPDU and APDU are just parts of the LPDU.
- **Layers nested by value in one task.** The application layer owns the transport layer, which owns the network layer, and so on. This is correct for embassy: it avoids a channel and a task per layer.

---

## 2. Correctness bugs

| ID | Location | Problem | Fix | Status |
|---|---|---|---|---|
| B1 | `transport_layer.rs:148-161` | The `RefMut` from `self.connection.borrow_mut()` is created inside the `select(...)` in the `match` expression, so it lives until the end of the whole `match` statement. Both arms call `borrow_mut()` again, which panics with `BorrowMutError`. Any T_Connect, T_Disconnect, T_Data_Connected, T_ACK or T_NAK panics. Group traffic is unaffected. | Compute the timeout future without keeping a borrow, e.g. store an `Option<Instant>` deadline, copy it out, then `select(..., Timer::at(d))`. | Resolved: deadline copied out before the `select`. Test: `transport_layer::tests::connect_then_data_is_acked` |
| B2 | `transport_layer.rs:106,112` | `tpci & 0x3c == 0xc2` and `== 0xc4` can never be true, so T_ACK and T_NAK are never recognised. | T_ACK: `tpci & 0xC3 == 0xC2`. T_NAK: `tpci & 0xC3 == 0xC3`. | Resolved. Test: `transport_layer::tests::t_ack_and_t_nak_are_recognised` |
| B3 | `transport_layer.rs:256,373` | `seq_no_recv - 1` underflows at 0, which panics in debug builds. `seq_no_recv + 1` does not wrap. | Sequence numbers are 4-bit: use `wrapping_add/sub(1) & 0xF`. Same for `seq_no_send`. | Resolved. `tpci_seq`/`set_tpci_seq` also masked only 3 bits; now 4. Tests: `transport_layer::tests::sequence_number_wraps_after_15`, `out_of_sequence_data_right_after_connect`, `frame::tests::sequence_number_has_four_bits` |
| B4 | `frame.rs:163`, `ExtendedFrame` impl (`frame.rs:379-392`) | `LG_FIELD` defaults to 5 and `ExtendedFrame` does not override it. In extended frames the length is at byte 6, so `length()` reads the low byte of the destination address. `length()` also overflows `u8` for long frames. | Override `LG_FIELD = 6` for extended frames. Return `usize` from `length()`. | Resolved. Tests: `frame::tests::parses_extended_frame`, `long_extended_frame_length` |
| B5 | `frame.rs:336,399-400` | `length <= MAX \|\| length >= MIN` is always true. The extended `set_length` writes `self.0.len()` instead of `length`. | Use `&&` and write `length`. | Resolved. Tests: `frame::tests::standard_frame_length_limits`, `extended_set_length_writes_requested_length` |
| B6 | `frame.rs:283-285` | `from_datapoint` always writes a 1-byte slice at `APCI_OFFSET + 1 + size`. Correct for 0 or 1 data bytes, wrong for multi-byte DPTs. | Write to `APCI_OFFSET+1` for data of 6 bits or less, otherwise to `APCI_OFFSET+2 .. APCI_OFFSET+2+size`. | Resolved; `U16` (DPT 7) added. Test: `frame::tests::frame_from_byte_datapoints` |
| B7 | `ncn51_driver.rs:273-283` | Every group telegram on the bus gets an ACK. This hides missing or broken receivers from senders. | ACK only the device's own individual address and group addresses in the group address table (see section 4, Driver ↔ stack). | Open |
| B8 | `application_layer.rs:89` | The `select` is not cancel-safe. `transport.receive()` sends frames inside itself (T_ACK) and waits for the confirmation. If `rx.ready_to_receive()` fires part-way, that future is dropped: the frame has already been taken from the queue, the ACK may not have been sent, and the sequence numbers are not updated. | `select` only takes the next event off a queue. All processing happens after `select` returns (see section 4). | Open |
| B9 | `ncn51_driver.rs:162`, `data_link_layer.rs:32-36` | The driver `await`s on the upward channel while the stack `await`s `CON_SIGNAL` from the driver. Today the pool (8 blocks, against 8+8 channel slots) runs out first, so frames are silently lost without an ACK. If the pool or channel sizes change, this becomes a deadlock. | The driver never awaits upper layers: use `try_send`, drop on full, and count the drops. | Open |
| B10 | `frame.rs` `set_frame_type` | Found by the host tests. New frames got control field `0x90` (standard) or no frame type at all (extended): bit 5 was 0, which marks a *repetition*. Every telegram the device sent was flagged as repeated, so a receiver that had seen an identical earlier telegram would discard it. | Set bit 5 (`0xB0` standard, `0x30` extended) and keep the priority bits. | Resolved. Tests: `frame::tests::new_frames_are_not_marked_repeated`, `builds_group_frame_with_setters` |
| B11 | `network_layer.rs` `receive`, driver | Found while writing the tests. The driver passes every received frame up, and the network layer indicates every individually addressed frame as `DataIndividual` without comparing the destination with our individual address. The transport layer therefore runs its connection state machine on connections between *other* devices (e.g. ETS ↔ 1.1.5) and answers them with T_ACK/T_Disconnect from our address. | Filter individually addressed frames on our address in the data link or network layer (together with the address filter of B7 and the runtime address of S2). | Open |
| B12 | `ncn51_driver.rs` `send_frame` | Found while doing Z3. The index in U_L_DataContinue/End has 6 bits, so frames longer than 64 octets need U_L_DataOffset.req (`0x08`, defined but unused) to set the upper index bits (NCN5130 datasheet, Send Frame Service: 3 MSBs of a 9-bit index; offsets 5–7 are forbidden; U_L_DataStart resets the offset). The driver ORs the full index into the command, so octet 64 onwards of an extended frame gets a wrong command byte. The simulator (`Ncn5130Sim::transmit`) has the same gap. | Send U_L_DataOffset before each 64-octet block; teach the simulator the command and add a test with a long extended frame. | Open |
| B13 | `ncn51_driver.rs` `send_frame` | Found while doing Z2. The NCN5130 echoes every attempt to send a frame (up to NACK + BUSY + 1 = 7 by default, repetitions with the repeat bit cleared), and a frame from a device that wins the arbitration arrives as a normal received frame before our echo. The driver read exactly one frame's worth of octets as the echo and then expected L_Data.con: after a repetition it reported a failed send, and a foreign frame was written into the TX buffer and lost. The wait for L_Data.con had no timeout, and the echo used the 2.6 ms inter-octet timeout for its first octet, although the transceiver only starts once the bus is free. | Until L_Data.con, treat each frame start as either an echo (same header) or a received frame; one 3 s timeout for the whole confirmation. | Resolved with Z2. Tests: `tests/driver.rs` `confirms_after_repetitions`, `receives_frame_while_waiting_for_confirmation` |

Minor issues:

- **M1** (`frame.rs:241-311`): the `set_*` methods use `|=`, so they only work on a zeroed buffer. Setting a field twice corrupts it. Use mask-and-set. *Resolved (`set_bits`). Test: `frame::tests::setting_a_field_twice_overwrites_it`.*
- **M2** (`ncn51_driver.rs:251`): the checksum check reads `data()[bytes_read]`, one byte past the checksum, which happens to be zero. It only works because the XOR over the whole frame including the checksum is 0. It panics for a full-length extended frame. Check explicitly that the XOR of all bytes including the checksum is `0xFF`. *Resolved. Test: `tests/driver.rs` `receives_full_length_extended_frame`.*
- **M3** (`data_link_layer.rs:35`): the L_Data.con status is discarded (`let con`), so failed sends are never reported upward. *Open. Since Z2 the status reaches the network and transport layers together with the frame, but nothing acts on it yet.*
- **M4** (`frame.rs` `apdu_data`): the slice runs up to `length()`, which counts the checksum octet, so it ends with the checksum. It starts at the octet shared with the low APCI bits. Unused so far. *Open.*

---

## 3. Memory and zero-copy

Target: nRF5340 network core, 256 KB flash and 64 KB RAM (`memory.x`).

| ID | Finding | Recommendation | Status |
|---|---|---|---|
| Z1 | **Pool block size.** The pool has 8 blocks of `Vec<u8, 264>`, about 2.1 KB (`ncn51_driver.rs:95-97`). Typical TP frames are 9–23 bytes, so over 90% of each block is unused. `StandardFrame` and `ExtendedFrame` share one pool, so splitting them into two types saves nothing and costs about 150 lines of forwarding `match`es (`frame.rs:443-577`). | Use one `Frame` type that adds 1 to its offsets at runtime when extended. Back it with a small pool (`[u8; 23]` + len, ×12 ≈ 300 B). Add a separate large pool of 1–2 blocks only if extended frames are needed. Declaring a maximum APDU length of 15 means ETS never sends extended frames. | Open |
| Z2 | **Clones in the transport layer.** `Frame::try_from(&frame)` copies the frame for possible repetition (`transport_layer.rs:433,454`), using two pool blocks per connected send. The copy is only needed because `send_frame` overwrites the TX buffer with the echo (`ncn51_driver.rs:211-214`). | Compare the echo byte by byte against the TX buffer instead of overwriting it; this also detects collisions. Return the untouched frame with the confirmation, `Signal<(Frame, ConStatus)>` (the KNX L_Data.con carries the frame). The transport layer keeps it for repeats: zero copies, one block. | Resolved. The driver writes the checksum into the frame's last octet, compares the echo against the frame instead of reading into it, and returns the frame with the confirmation (`Bus::con: Signal<(Frame, ConStatus)>`, `DataLinkLayer::send`/`NetworkLayer::send` return it). The transport layer keeps the returned frame for repeats; the `TryFrom<&Frame>` clones are gone. Echoes are recognised by their header (repeat bit masked) and compared without a pool block and without an ACK. Tests: `tests/driver.rs` `confirmation_returns_the_sent_frame`, `confirms_after_repetitions`, `receives_frame_while_waiting_for_confirmation` |
| Z3 | **Per-byte DMA.** Every RX and TX byte is its own 1-byte UARTE DMA transfer, and every RX byte registers a timer. A 23-byte TX takes about 48 transfers. When the `select` in `ncn51_driver.rs:149` drops `uarte.read()`, the RX DMA is stopped and a frame start byte arriving in that window can be lost. | TX: build the `(cmd, byte)` pairs in a buffer on the stack (≤48 B) and send it with a single `write`. RX: use `embassy_nrf::buffered_uarte::BufferedUarte` + `split()`; reception runs continuously into a ring buffer (TIMER+PPI byte counting) and frames are parsed from it. | Resolved. TX: the command stream goes out in one `write` per 64 bytes (one for any standard frame). RX: the firmware uses `BufferedUarte` (TIMER2, PPI_CH0/1, PPI_GROUP0, 256 B ring buffer) as a single `Read + Write` object, without `split()`, since one task owns both directions. The driver reads as many octets per call as are available, with one timer per call; on the bus octets arrive about 1 ms apart, so that is still about one timer per octet, but no DMA restart and no lost bytes. Test: `tests/driver.rs` `transmits_standard_frame_in_one_write` |
| Z4 | **Task arena.** `task-arena-size-32768` (`Cargo.toml:9`) reserves half of RAM. | Measure the task future sizes (`core::mem::size_of_val` on each task's future, or `-Zprint-type-sizes`) and shrink the arena to fit with some headroom. | Resolved by the upgrade to embassy-executor 0.10, which allocates tasks statically (no arena). Measured release build: `.data` + `.bss` + `.uninit` ≈ 6 KB |
| Z5 | **`DataPoint` enum as the application interface.** Every variant is as large as the biggest one, and it is copied by value through channels. That cost grows with DPT 16 (14 B) and variable-length strings. | Store group object values as bytes in one static buffer. `DataPoint`/DPT types become typed codecs used by the application (see section 4, Stack ↔ application). | Open |
| Z6 | **Build profiles.** No size optimisation is configured. Unoptimised async code is slow for a 2.6 ms inter-byte budget. | Release: `opt-level = "s"`, `lto = "fat"`, `codegen-units = 1`. Dev: `opt-level = 1` or higher. | Resolved in `firmware/Cargo.toml` as recommended (dev `opt-level = 1`). Flash (`.text` + `.rodata`): dev 130 → 51 KB, release 40.7 → 29.4 KB |

---

## 4. Task (thread) architecture

In embassy, "threads" means tasks on an executor. A task only preempts another if it runs on an `InterruptExecutor` at a higher priority. At the reviewed commit there are three tasks (`uart_task`, `application_task`, `main`) on one thread-mode executor, and every primitive uses `ThreadModeRawMutex`.

### Target design

```
 ┌─ high prio (InterruptExecutor on a SWI/EGU IRQ) ─┐
 │ ncn5130 task                                      │
 │  BufferedUarte RX/TX, frame assembly, U_State,   │──BusRx: Channel<Frame, N>──┐
 │  reset/init, L_Data.req → .con (returns Frame)   │◀─BusTx: Channel<Frame, 1>──┤
 │  ACK decision: synchronous, no await             │                            │
 │  NEVER awaits on upper layers (try_send + count) │                            │
 └───────────────▲──────────────────────────────────┘                            │
                 │ reads (sync)                                                  │
       AddressFilter: IA + GA table (blocking Mutex / atomics)                   │
                                                                                 │
 ┌─ thread mode ─────────────────────────────────────────────────────────────────▼┐
 │ stack task: DL (filter, repetition dedupe) → NL → TL (FSM + timers) → AL        │
 │             → GO server, management server (properties/memory)                  │
 │ loop { select(bus_rx.receive(), go_tx_pending.wait(), tl_deadline) → handle }   │
 └──────────────┬───────────────────────────────────────────▲──────────────────────┘
                │ GO table: values + flags (blocking Mutex)  │ transmit request (Signal)
 ┌──────────────▼───────────────────────────────────────────┴───┐   ┌─────────────────┐
 │ application task(s): go.get::<Dpt9>() / go.set(v)            │   │ persistence     │
 └──────────────────────────────────────────────────────────────┘   │ (flash, bus idle)│
                                                                    └─────────────────┘
```

### Boundary rules

**Driver ↔ stack** (the only hard real-time boundary)
- The `U_AckInformation` decision must be made inside the NCN5130 ACK window, so it cannot wait on a channel round trip. The driver reads the address filter (individual address + group address table) synchronously, from shared state behind a blocking mutex or atomics.
- The driver never `await`s on anything above it. When the queue is full it drops the frame and counts the drop (fixes B9).
- Run the driver on a higher-priority `InterruptExecutor` so that management or application work cannot delay the ACK. This needs every primitive shared with the driver to use `CriticalSectionRawMutex`, because `ThreadModeRawMutex` panics outside thread mode.
- Name the channels from the bus's point of view: `BUS_RX` / `BUS_TX`. At the reviewed commit, `FRAME_CHANNEL_TX` carries frames *from* the bus (`ncn51_driver.rs:101-102`). *Done: `Bus { bus_rx, bus_tx, con }` in `src/bus.rs`.*
- The driver owns the NCN5130 control plane: reset and initialisation at startup, U_State.ind (thermal warning, protocol/transceiver errors), and U_SystemStat.

**Inside the stack** (one task, layers nested by value)
- Keep one task with the layers nested by value. The only timing needs inside the stack are the transport timers.
- Event-loop rule: `select` only takes the next event off a queue (bus RX, group-object transmit request, timer deadline). All processing, including waiting for L_Data.con, happens after `select` returns. This makes every event source cancel-safe (fixes B8).
- Store transport timeouts as deadlines (`Option<Instant>` + `Timer::at`), not as a `Ticker`. Both the 6 s connection timeout and the 3 s T_ACK timeout are needed.

**Stack ↔ application** (group object model, not a request/response channel)
- At the reviewed commit, `main.rs:84-98` answers GroupValueRead through a round trip to the user task. Use the standard KNX device model instead:
  - The stack answers GroupValueRead directly from the group object value when the Read flag is set.
  - GroupValueWrite updates the value when the Write flag is set, sets the Update flag, and wakes the application.
  - `go.set(v)` sets Transmit-request and wakes the stack through a `Signal`.
- Store the values as bytes in one static buffer, with offsets from the group object table. Each direction then does exactly one, unavoidable, copy between the APDU and the group object storage.

**Persistence**
- On the nRF5340 network core, flash erase and write stall the CPU, and an interrupt executor does not help. Keep tables and parameters in RAM and write them to flash while the bus is idle, for example after the ETS connection closes.

---

## 5. Other structural findings

| ID | Finding | Status |
|---|---|---|
| S1 | **Dependency inversion.** `frame.rs:2` imports `FRAME_POOL` from the driver. Move the pool into the frame module or the stack. | Resolved: pool and `init_frame_pool()` in `frame.rs` |
| S2 | **Compile-time individual address.** `settings::ADDRESS` is a `const` used from the driver and the transport layer. It must be settable at runtime (programming mode, ETS). Move it into a runtime device-state struct together with the group address, association and group object tables. | Open |
| S3 | **No host testing.** Everything depends on nRF types. Make the driver generic over `embedded_io_async::Read + Write`, and keep frame, transport and application code HAL-free, so the frame parser and the transport state machine can be unit-tested on the host. Such tests would have caught B1–B6. | Resolved: library/firmware split, driver generic over `embedded_io_async`, host tests (see `docs/testing.md`) |
| S4 | **Unused dependencies:** `ambassador`, `bit_field`, `embedded-io-async` (until S3 is done). | Resolved: `ambassador` and `bit_field` dropped; `embedded-io-async` is now used |
| S5 | **Logging in the byte path.** `info!` in the driver's per-byte path (e.g. "Unexpected byte") can stall timing-critical code, because probe-rs may put RTT in blocking mode. The per-frame `info!` in `data_link_layer.rs:41` should also be removed or demoted outside development. | Open |
| S6 | **Hardcoded addressing.** The destination group address is hardcoded (`transport_layer.rs:72`), as is ASAP 0 (`application_layer.rs:69`, `main.rs:89`). Both should come from the association table. | Open |

---

## 6. Open questions for the project owner

These were asked and not yet answered at review time. Do not guess the answers. If an answer is recorded here later, update the recommendations above that depend on it.

1. **Library or product?** Is `embassy-knx` a reusable crate (generic over transceiver and HAL) or firmware for one board? This decides how much abstraction S3 should add.
2. **Why the nRF5340 network core?** Is BLE or Thread planned there, which would compete for its 64 KB of RAM and CPU time? Is the application core used? Splitting stack and application across the cores would move the application boundary to IPC.
3. **Feature scope.** Does the device need to be fully programmable from ETS (connection-oriented transport, property and memory services, device descriptor, persistence)? Is KNX Secure planned (AES-CCM, more RAM, longer frames)? What maximum APDU length will be declared? The answer decides Z1 (whether extended frames and a large pool are needed).
4. **Size.** Roughly how many group objects and group addresses are expected? This sizes the tables, the filter lookup (linear scan vs. sorted array with binary search) and the frame pool.

---

## 7. Suggested order of work

1. Bugs B1–B7 (small, local fixes).
2. Driver rework: Z3 (`BufferedUarte` split, single-transfer TX), Z2 (echo compare, confirmation returns the frame), B7 (ACK filter), B9 (never block upward), reset/init/U_State.
3. Z1: merge the frame types into one and resize the pool. Fix M1 and M2 along the way.
4. B8: restructure the stack's event loop and switch timers to deadlines.
5. Stack ↔ application: group object table with flags as the application interface; S2 runtime device state; persistence.
6. Z4: measure future sizes and shrink the arena. Then move the driver to an `InterruptExecutor` if measured ACK latency requires it.
