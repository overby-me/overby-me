//! The ballot service's PERSISTENT layer (rewrite kickoff items 12 + 13): binds
//! `ballot-spec`'s pure, property-tested ballot core to a durable Turso store.
//!
//! Two halves, mirroring the scheme's public/private split:
//! - [`board`]: the DURABLE public bulletin board. `PersistentBoard::cast` runs
//!   the kill-9-proven atomic dedup+append transaction, with the entry body kept
//!   as an OPAQUE provisional blob (the byte encodings are item 9's decision).
//! - [`roster`]: the PRIVATE, org-authoritative eligibility / delegation /
//!   token-issuance tables and the freeze-at-open routine.
//!
//! Neither half needs the board-custody call or the D1-D8 sign-off: the board
//! rests only on the decided UNIQUE-token dedup, and the roster enforces the
//! already-property-tested D1-D3 rules (overturning one is a code+test change).

pub mod board;
pub mod replica;
pub mod roster;

pub use board::{BOARD_DDL, BoardError, PersistentBoard, ReplicationSink};
pub use replica::{RebuildError, ReplicaLog, rebuild_from_replica};
pub use roster::{BALLOT_DDL, freeze_at_open};
