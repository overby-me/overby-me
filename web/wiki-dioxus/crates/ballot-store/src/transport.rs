//! Off-node replica TRANSPORT: incrementally ship the append-only [`ReplicaLog`]
//! (`replica.rs`) to an INDEPENDENT follower node, WAL-shipping style. `replica.rs`
//! frames the artifact (a JSONL append-only file) and the rebuild path but leaves
//! the wire deliberately open; this module pins the concrete incremental
//! transport: a monotone BYTE-OFFSET cursor, ship-only-whole-records (never a torn
//! tail), resumable across restarts, and idempotent. A recovery test rebuilds a
//! board from the shipped downstream copy to prove the follower is a faithful
//! replica, not just an approximate one.
//!
//! The transport is intentionally dumb: bytes in append order, and a cursor for
//! how far the follower has consumed. It works over ANY byte pipe (rsync, scp, an
//! HTTP range GET, a socket) -- nothing here binds a network stack, matching the
//! decision that the off-node integrity control must not depend on one provider.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// How far a follower has consumed the source log, as a byte offset. Persist this
/// on the follower so shipping resumes across restarts; it only ever advances, and
/// only ever by whole records, so it can also be compared to detect divergence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipCursor {
    pub offset: u64,
}

/// One incremental ship: the whole-record bytes to append downstream and the
/// advanced cursor. `bytes` is empty (and `cursor` is unchanged) when the follower
/// is already caught up or only a torn tail is available.
#[derive(Debug, Clone)]
pub struct ShipBatch {
    pub bytes: Vec<u8>,
    pub cursor: ShipCursor,
}

impl ShipBatch {
    /// Whether this batch carried any records (false = follower already current).
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Read new WHOLE records from `source` starting at `from.offset`. Stops at the
/// last complete line (a JSONL record ends in `\n`), so a concurrently-growing log
/// caught mid-append never ships a TORN record; the partial tail ships on the next
/// call once its newline lands. The cursor advances only by the bytes actually
/// emitted -- never into the torn tail -- which is what makes a follower loop
/// safe to run against a live, still-appending primary.
pub fn read_incremental(source: impl AsRef<Path>, from: ShipCursor) -> std::io::Result<ShipBatch> {
    let mut file = File::open(source)?;
    let len = file.metadata()?.len();
    // Already caught up (or the primary was truncated/rotated under us: don't
    // rewind, a monotone cursor never goes backwards).
    if from.offset >= len {
        return Ok(ShipBatch {
            bytes: Vec::new(),
            cursor: from,
        });
    }
    file.seek(SeekFrom::Start(from.offset))?;
    let mut buf = Vec::with_capacity((len - from.offset) as usize);
    file.read_to_end(&mut buf)?;
    // Trim to the last complete record: everything up to and including the last
    // newline. If there is no newline yet, nothing is complete.
    let complete = match buf.iter().rposition(|&b| b == b'\n') {
        Some(i) => i + 1,
        None => 0,
    };
    buf.truncate(complete);
    Ok(ShipBatch {
        bytes: buf,
        cursor: ShipCursor {
            offset: from.offset + complete as u64,
        },
    })
}

/// Append a shipped batch to the FOLLOWER's replica log at `dest` (append-only,
/// created if absent). Returns the bytes appended. Idempotent given a monotone
/// cursor: the caller advances its persisted cursor by the batch, so the same
/// bytes are never applied twice.
pub fn apply_batch(dest: impl AsRef<Path>, batch: &ShipBatch) -> std::io::Result<usize> {
    if batch.bytes.is_empty() {
        return Ok(0);
    }
    let mut file = OpenOptions::new().create(true).append(true).open(dest)?;
    file.write_all(&batch.bytes)?;
    file.flush()?;
    Ok(batch.bytes.len())
}

/// One catch-up pass: ship every currently-available whole record from `source` to
/// the follower log `dest`, returning the advanced cursor to persist. A follower
/// runs this on a timer or a change notification; the returned cursor is fed back
/// on the next call. The follower's log is byte-identical to the primary's
/// committed prefix, so [`crate::replica::rebuild_from_replica`] over it yields the
/// original board.
pub fn ship(
    source: impl AsRef<Path>,
    dest: impl AsRef<Path>,
    cursor: ShipCursor,
) -> std::io::Result<ShipCursor> {
    let batch = read_incremental(source, cursor)?;
    apply_batch(dest, &batch)?;
    Ok(batch.cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::PersistentBoard;
    use crate::replica::{ReplicaLog, rebuild_from_replica};
    use ballot_spec::{BallotRules, BoardEntry, TokenIssuer, finalize_token, request_token};
    use std::sync::Arc;

    fn rules() -> BallotRules {
        BallotRules {
            options: 3,
            blank: false,
            min: 1,
            max: 1,
        }
    }

    fn valid_entry(issuer: &TokenIssuer, choices: Vec<usize>) -> BoardEntry {
        let pk = issuer.public_key();
        let req = request_token(pk).expect("request");
        let blind_sig = issuer
            .blind_sign(&req.blinding.blind_message)
            .expect("blind sign");
        let signature = finalize_token(pk, &req, &blind_sig).expect("finalize");
        BoardEntry {
            token: req.nullifier,
            msg_randomizer: req.blinding.msg_randomizer,
            signature,
            choices,
        }
    }

    async fn fresh_board() -> PersistentBoard {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build");
        PersistentBoard::open(db.connect().expect("connect"))
            .await
            .expect("open")
    }

    #[test]
    fn torn_tail_is_never_shipped() {
        let src =
            std::env::temp_dir().join(format!("ballot-ship-torn-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&src);
        // Two complete records and a torn third (no trailing newline yet).
        std::fs::write(&src, b"{\"a\":1}\n{\"a\":2}\n{\"a\":3").expect("seed");

        let batch = read_incremental(&src, ShipCursor::default()).expect("read");
        assert_eq!(
            batch.bytes, b"{\"a\":1}\n{\"a\":2}\n",
            "only whole records ship"
        );
        assert_eq!(batch.cursor.offset, 16, "cursor stops before the torn tail");

        // The torn record completes; the next pass ships exactly it.
        std::fs::write(&src, b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n").expect("complete");
        let batch2 = read_incremental(&src, batch.cursor).expect("read2");
        assert_eq!(
            batch2.bytes, b"{\"a\":3}\n",
            "the completed record ships next"
        );

        // Caught up: an empty batch, cursor unchanged.
        let batch3 = read_incremental(&src, batch2.cursor).expect("read3");
        assert!(batch3.is_empty());
        assert_eq!(batch3.cursor, batch2.cursor);

        let _ = std::fs::remove_file(&src);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn incremental_shipping_rebuilds_the_board_on_the_follower() {
        let pid = std::process::id();
        let src = std::env::temp_dir().join(format!("ballot-ship-src-{pid}.jsonl"));
        let dst = std::env::temp_dir().join(format!("ballot-ship-dst-{pid}.jsonl"));
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);

        // Primary board shipping its casts into the source replica log.
        let replica = Arc::new(ReplicaLog::open(&src).expect("open replica"));
        let primary = fresh_board().await.with_replication(replica.clone());
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");

        // Round 1: two casts, then a partial ship to the follower.
        for choice in [0usize, 1] {
            primary
                .cast(
                    issuer.public_key(),
                    &rules(),
                    valid_entry(&issuer, vec![choice]),
                )
                .await
                .expect("cast");
        }
        let mut cursor = ShipCursor::default();
        cursor = ship(&src, &dst, cursor).expect("ship round 1");
        assert!(
            cursor.offset > 0,
            "the follower consumed the first two records"
        );

        // Round 2: a third cast arrives; the follower, resuming from its persisted
        // cursor, ships ONLY the new record (WAL-style incremental, no re-send).
        primary
            .cast(issuer.public_key(), &rules(), valid_entry(&issuer, vec![2]))
            .await
            .expect("cast 3");
        let before = cursor;
        cursor = ship(&src, &dst, cursor).expect("ship round 2");
        assert!(
            cursor.offset > before.offset,
            "cursor advanced by the new record only"
        );

        // A redundant ship with no new data is a no-op (idempotent follower loop).
        let steady = ship(&src, &dst, cursor).expect("ship steady");
        assert_eq!(steady, cursor, "no new records => cursor holds");

        // Recovery: rebuild a fresh board purely from the FOLLOWER's copy. It must
        // equal the primary byte-for-byte, proving the shipped log is a faithful,
        // complete replica -- the off-node integrity guarantee.
        let original = primary.entries().await.expect("primary entries");
        assert_eq!(original.len(), 3);
        let rebuilt = fresh_board().await;
        let restored = rebuild_from_replica(&rebuilt, &dst).await.expect("rebuild");
        assert_eq!(restored, 3, "every shipped entry restored on the follower");
        assert_eq!(
            rebuilt.entries().await.expect("rebuilt entries"),
            original,
            "the follower's board == the primary's board"
        );

        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }
}
