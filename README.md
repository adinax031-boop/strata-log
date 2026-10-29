# Strata

**A small, inspectable append-only event log in Rust.** Strata stores atomic batches in checksummed frames and recovers a torn final write without silently accepting a complete corrupt frame. No dependencies and no unsafe Rust.

```sh
cargo test
cargo run -- append demo.strata "engine started" "order accepted"
cargo run -- verify demo.strata
cargo run -- dump demo.strata   # payloads printed as hex
```

```rust
use strata_log::{Durability, Log};
let mut log = Log::open("events.strata")?;
let seq = log.append(&[b"created".to_vec(), b"approved".to_vec()], Durability::Sync)?;
assert_eq!(seq, 0);
# Ok::<(), strata_log::Error>(())
```

## Contract

- A batch is one length-delimited frame with a monotonic sequence and CRC-32 over metadata and payload. Recovery exposes a whole batch or none of it.
- An incomplete tail frame is truncated during open; a complete frame with invalid checksum, format, or sequence fails closed. A truncated payload cannot be distinguished from certain corruption of its length field. CRC-32 detects accidental corruption, not malicious tampering.
- `Sync` calls `sync_data` before returning; `Buffered` does not. Actual crash durability depends on the operating system, filesystem and hardware. File creation syncs the file but does not sync its parent directory, so existence after power loss is not guaranteed.
- A write or sync failure poisons the handle because commit status is uncertain. Reopen and inspect before retrying; a failed append may have been committed.
- **Single writer only:** the caller must enforce exclusive access across processes and handles. No locking is implemented. Never open a live log concurrently, including for reading: open may truncate an incomplete tail.
- Opening scans the entire log and holds all records in memory. This is a reference implementation, not a high-throughput database; no compaction, replication, authentication, async API, or adversarial-input hardening is claimed. Maximum frame payload is 64 MiB.
- `verify` opens the file and can truncate a partial tail; it creates a missing log. It is not a read-only forensic tool.

## Format v1

All integers are little endian. File header is `STRATA01`. Each frame is `BAT1`, payload length u32, sequence u64, record count u32, CRC-32 u32, then payload. Payload consists of repeated u32 length + bytes. CRC is IEEE CRC-32 over the first 20 frame bytes followed by payload, excluding the checksum. Sequence starts at zero. Empty batches and empty records are supported.

## Development

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. The integration tests exercise every truncation point of a final frame, corruption, sequence continuity and reopening. MIT licensed.
