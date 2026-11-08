//! Content-addressed storage for the captured request and response bodies.
//!
//! The bodies are the largest thing in the database by two orders of magnitude,
//! and the reason is structural rather than accidental: a chat client resends
//! the whole conversation on every turn, so a long session posts the same
//! growing prefix thousands of times and each copy is stored in full. Measured
//! on this install, one session had 12,347 turns and two sessions held 63% of
//! every request ever logged.
//!
//! So a body is not stored as one value. It is split into content-defined
//! chunks, and each chunk is stored once, keyed by its own hash. A turn that
//! only appends a few kilobytes reuses every chunk of the history it repeats;
//! only the new tail is new data. Chunks are compressed individually, because
//! a chunk's bytes are the same bytes every time they appear — compressing
//! across turns buys nothing beyond what deduplication already bought, and it
//! would make a chunk undecodable on its own.
//!
//! Chunk boundaries are content-defined (a rolling hash over the bytes) rather
//! than fixed-size. Fixed windows look tempting — they are cheaper — but an
//! insertion near the start of a body shifts every later window by one byte and
//! the whole body stops matching. Content-defined boundaries re-synchronise a
//! few bytes after the edit, which is the entire point when the edit is
//! "somebody typed one character into the first message".
//!
//! # Why the layout is a blob and not a table
//!
//! What a row stores is the ordered list of chunk hashes — sixteen bytes each,
//! concatenated. A link table would make garbage collection pure SQL, but it
//! also costs a row, an index entry and a page-fragmentation share per chunk,
//! which at three million chunks is more than the layouts themselves. The blob
//! is compact, and the sweep that finds unreferenced chunks reads every layout
//! once (tens of megabytes) instead of joining a table of millions of rows.
//!
//! # Garbage
//!
//! Chunks are never reference-counted. A counter has to be kept exactly right
//! across every delete path that exists now and every one added later, and a
//! drift in either direction is either a leak that never heals or a chunk
//! deleted out from under a live row. Instead a sweep marks every hash reachable
//! from a stored layout and deletes the rest — correct by construction, and
//! cheap enough to run after every prune.

use crate::error::Result;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Target chunk size. Chosen by measuring this install's real bodies: at 8 KiB
/// the deduplicated and compressed result was 39x smaller than the raw bytes,
/// counting the layout overhead; 4 KiB was marginally worse (more per-row
/// hashes) and 16 KiB noticeably worse (less of a session matched).
const CHUNK_AVG: usize = 8 * 1024;

/// Minimum and maximum chunk length, as multiples of the average. The minimum
/// stops a long run of "cut here" bytes from producing a flood of tiny chunks;
/// the maximum bounds what a run with no cut point can cost.
const CHUNK_MIN: usize = CHUNK_AVG / 4;
const CHUNK_MAX: usize = CHUNK_AVG * 4;

/// Length of a stored hash, in bytes. A truncated SHA-256: 128 bits is far more
/// than enough to keep three million chunks apart, and every byte here is paid
/// once per chunk reference.
const HASH_LEN: usize = 16;

/// zstd level for a chunk. Chunks are small, so the level changes the ratio
/// little and the time per chunk matters more — this runs on the write path of
/// every logged request.
const ZSTD_LEVEL: i32 = 6;

/// The gear table: one pseudo-random 64-bit constant per byte value.
///
/// Built once from a fixed seed rather than shipped as a literal table, so this
/// file stays readable and the constants cannot drift between runs — the same
/// bytes must always produce the same chunk boundaries, or a body written by an
/// older build would stop matching a body written by a newer one.
fn gear_table() -> &'static [u64; 256] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u64; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u64; 256];
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for slot in t.iter_mut() {
            // splitmix64: a well-distributed cheap generator.
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            *slot = z ^ (z >> 31);
        }
        t
    })
}

fn hash_of(bytes: &[u8]) -> [u8; HASH_LEN] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; HASH_LEN];
    out.copy_from_slice(&digest[..HASH_LEN]);
    out
}

/// Split a body into content-defined chunks.
///
/// The cut test is the top bits of a rolling hash over the bytes since the last
/// cut: every byte contributes, so any insertion changes the region around it
/// and leaves the boundaries beyond it alone. Measured on this install's real
/// request bodies, a body with one byte inserted at its head shares 98-100% of
/// its chunks with the original.
///
/// The shift-and-add form with no eviction means the hash remembers the last 64
/// bytes, not all of them. That is the intended behaviour — it is what makes a
/// boundary survive an edit — but it has one pathological input: a body whose
/// bytes repeat with a short period drives the hash into a cycle, and a cycle
/// that never lands on the cut pattern makes every chunk hit the maximum. Runs
/// of a single repeated byte hit that; so does a file of same-width rows. JSON
/// request bodies do not, and that is what is stored here, but the maximum
/// bound is what keeps the case merely wasteful instead of unbounded.
fn split(bytes: &[u8]) -> Vec<&[u8]> {
    let table = gear_table();
    // Cut when the top `bits` bits are all zero: one hit per `CHUNK_AVG` bytes
    // on average, for input whose hash is uniform.
    let shift = 64 - CHUNK_AVG.trailing_zeros();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut h: u64 = 0;

    for (i, byte) in bytes.iter().enumerate() {
        h = (h << 1).wrapping_add(table[*byte as usize]);
        let len = i - start + 1;
        if len >= CHUNK_MIN && ((h >> shift) == 0 || len >= CHUNK_MAX) {
            out.push(&bytes[start..=i]);
            start = i + 1;
            h = 0;
        }
    }
    if start < bytes.len() {
        out.push(&bytes[start..]);
    }
    out
}

/// Store one body, returning the layout blob that stands for it.
///
/// Must run inside the caller's transaction: the body row and its chunks have
/// to appear together, or a crash between the two leaves a layout naming chunks
/// that were never written.
pub fn store(tx: &Connection, text: &str) -> Result<Vec<u8>> {
    let bytes = text.as_bytes();
    // `OR IGNORE` because a repeated chunk is the normal case, not an error:
    // it is exactly what the whole scheme is for. The stored copy wins — the
    // bytes are identical by construction, since the hash is of the bytes.
    let mut insert =
        tx.prepare_cached("INSERT OR IGNORE INTO body_chunks (hash, data) VALUES (?1, ?2)")?;
    let mut layout = Vec::with_capacity(bytes.len() / CHUNK_AVG * HASH_LEN + HASH_LEN);

    for chunk in split(bytes) {
        let hash = hash_of(chunk);
        let packed = zstd::bulk::compress(chunk, ZSTD_LEVEL)?;
        insert.execute(params![&hash[..], packed])?;
        layout.extend_from_slice(&hash);
    }
    Ok(layout)
}

/// Reads layouts back into text, caching chunks across calls.
///
/// One reader serves a whole export: the same chunks appear in row after row,
/// and the cache turns what would be millions of lookups into one per distinct
/// chunk. The cache is bounded because an export can walk more distinct chunks
/// than the process should hold — every entry is an uncompressed chunk, so
/// without a ceiling a 100,000-row export would keep gigabytes alive. Past the
/// ceiling it clears rather than evicts one at a time: a full export reads rows
/// in order, so what it held is mostly behind it, and the old entries cost more
/// to find than to drop.
pub struct BodyReader<'a> {
    conn: &'a Connection,
    cache: HashMap<[u8; HASH_LEN], Option<Vec<u8>>>,
    cached_bytes: usize,
}

/// How many uncompressed chunk bytes the cache may hold.
const CACHE_BUDGET: usize = 64 * 1024 * 1024;

impl<'a> BodyReader<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self {
            conn,
            cache: HashMap::new(),
            cached_bytes: 0,
        }
    }

    /// Rebuild one body from its layout. An empty layout is an empty body —
    /// which is not the same thing as `None`, the "no body stored at all" a
    /// pre-forward failure has.
    pub fn read(&mut self, layout: &[u8]) -> Result<String> {
        let mut bytes = Vec::new();
        for key in layout.as_chunks::<HASH_LEN>().0 {
            let key = *key;

            if !self.cache.contains_key(&key) {
                let packed: Option<Vec<u8>> = self
                    .conn
                    .query_row(
                        "SELECT data FROM body_chunks WHERE hash = ?1",
                        params![&key[..]],
                        |r| r.get(0),
                    )
                    .ok();
                let unpacked = match &packed {
                    Some(p) => Some(zstd::stream::decode_all(&p[..])?),
                    None => None,
                };
                if let Some(chunk) = &unpacked {
                    if self.cached_bytes + chunk.len() > CACHE_BUDGET {
                        self.cache.clear();
                        self.cached_bytes = 0;
                    }
                    self.cached_bytes += chunk.len();
                }
                self.cache.insert(key, unpacked);
            }

            match &self.cache[&key] {
                Some(chunk) => bytes.extend_from_slice(chunk),
                // A chunk the sweep removed is not recoverable, and a body
                // missing a piece is worth less than nothing if it is handed
                // out as though it were whole. It is logged rather than
                // raised: the export must not die on one damaged row.
                None => tracing::warn!(
                    chunk = %hex(&key),
                    "body chunk missing; the rebuilt body will be incomplete"
                ),
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Collect every chunk hash still reachable from a stored body.
fn live_hashes(conn: &Connection) -> Result<std::collections::HashSet<[u8; HASH_LEN]>> {
    let mut live = std::collections::HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT request_layout, response_layout FROM request_bodies
         WHERE request_layout IS NOT NULL OR response_layout IS NOT NULL",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        for side in 0..2 {
            if let Some(layout) = row.get::<_, Option<Vec<u8>>>(side)? {
                for key in layout.as_chunks::<HASH_LEN>().0 {
                    live.insert(*key);
                }
            }
        }
    }
    Ok(live)
}

/// Delete every chunk no stored body refers to. Returns how many were removed.
///
/// Runs after anything that can orphan a chunk — a prune, a cleared log. It is
/// a full mark-and-sweep, so it is also the repair path: a chunk leaked by a
/// path that forgot to sweep is collected the next time any sweep runs.
///
/// The comparison happens in memory rather than in SQL, because "not in this
/// set of three million 16-byte values" is exactly the shape that makes SQLite
/// fall back to a scan per row. The live set is read first into a hash set, then
/// the chunk table is walked once and the dead hashes are deleted by primary
/// key in batches.
pub fn sweep(conn: &Connection) -> Result<usize> {
    let live = live_hashes(conn)?;

    let dead: Vec<Vec<u8>> = {
        let mut stmt = conn.prepare("SELECT hash FROM body_chunks")?;
        let mut rows = stmt.query([])?;
        let mut dead = Vec::new();
        while let Some(row) = rows.next()? {
            let hash: Vec<u8> = row.get(0)?;
            if !hash
                .as_slice()
                .try_into()
                .map(|k: [u8; HASH_LEN]| live.contains(&k))
                .unwrap_or(true)
            // an odd-length hash is not ours to delete
            {
                dead.push(hash);
            }
        }
        dead
    };

    let mut removed = 0;
    let mut delete = conn.prepare_cached("DELETE FROM body_chunks WHERE hash = ?1")?;
    for batch in dead.chunks(500) {
        for hash in batch {
            removed += delete.execute(params![hash])?;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE body_chunks (hash BLOB PRIMARY KEY, data BLOB NOT NULL) WITHOUT ROWID;
             CREATE TABLE request_bodies (
                 log_id        INTEGER PRIMARY KEY,
                 request_body  TEXT,
                 response_body TEXT,
                 request_layout  BLOB,
                 response_layout BLOB);",
        )
        .unwrap();
        c
    }

    /// A body shaped like the ones this actually stores: a JSON conversation
    /// whose every message is distinct. Generative, so the test needs no
    /// fixture, and varied enough to be honest about chunk boundaries — a body
    /// of one repeated line is a degenerate input the module documents but does
    /// not optimise for.
    fn chat_body(messages: usize, tail: &str) -> String {
        let msgs: Vec<String> = (0..messages)
            .map(|i| {
                format!(
                    r#"{{"role":"{}","content":[{{"type":"text","text":"turn {i}: {}"}}]}}"#,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod"
                )
            })
            .collect();
        format!(
            r#"{{"model":"deepseek/deepseek-v4.1-flash","messages":[{}],"tail":"{tail}"}}"#,
            msgs.join(",")
        )
    }

    #[test]
    fn round_trip_is_byte_exact() {
        let c = conn();
        let text = r#"{"model":"x","messages":[{"role":"user","content":"héllo ✨ 世界"}]}"#;
        let layout = store(&c, text).unwrap();
        let mut r = BodyReader::new(&c);
        assert_eq!(r.read(&layout).unwrap(), text);
    }

    #[test]
    fn empty_body_round_trips() {
        let c = conn();
        let layout = store(&c, "").unwrap();
        assert!(layout.is_empty());
        assert_eq!(BodyReader::new(&c).read(&layout).unwrap(), "");
    }

    /// The property the whole module exists for: appending to a session stores
    /// the new tail, not the history it repeats.
    #[test]
    fn appending_stores_only_the_tail() {
        let c = conn();
        let mut layouts = Vec::new();
        let mut sizes = Vec::new();
        for turn_no in 0..20 {
            // Each turn carries the whole history, exactly as a chat client
            // sends it, plus the one new message at the end.
            let body = chat_body(turn_no * 20, &format!("assistant turn {turn_no}"));
            sizes.push(body.len());
            layouts.push(store(&c, &body).unwrap());
        }
        let mut r = BodyReader::new(&c);
        assert_eq!(
            r.read(layouts.last().unwrap()).unwrap(),
            chat_body(19 * 20, "assistant turn 19")
        );

        // Storing each turn in full would be the sum of every size; the chunks
        // are the size of the last body plus what the earlier ones added.
        let stored: i64 = c
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(data)), 0) FROM body_chunks",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let full: usize = sizes.iter().sum();
        // Compression is in the mix, so this is not an exact ratio; what it
        // pins is that the history is not stored twenty times over.
        assert!(
            (stored as usize) * 4 < full,
            "20 turns stored {stored} bytes of {full} sent; the history did not dedupe"
        );
    }

    #[test]
    fn an_insertion_does_not_lose_every_later_chunk() {
        let c = conn();
        let body = chat_body(4000, "one");
        let a = store(&c, &body).unwrap();
        let before: i64 = c
            .query_row("SELECT COUNT(*) FROM body_chunks", [], |r| r.get(0))
            .unwrap();
        let chunks_per_body = (a.len() / HASH_LEN) as i64;

        // Insert one character near the start.
        let edited = format!("X{body}");
        let b = store(&c, &edited).unwrap();
        let after: i64 = c
            .query_row("SELECT COUNT(*) FROM body_chunks", [], |r| r.get(0))
            .unwrap();

        // Content-defined boundaries resynchronise within about a chunk of the
        // edit, so only the chunks around it are new — not the whole body, which
        // is what fixed-size windows would produce.
        let added = after - before;
        assert!(
            added < chunks_per_body / 2,
            "insertion added {added} of {chunks_per_body} chunks"
        );
        let mut r = BodyReader::new(&c);
        assert_eq!(r.read(&b).unwrap(), edited);
        assert_eq!(r.read(&a).unwrap(), body);
    }

    #[test]
    fn sweep_removes_only_unreferenced_chunks() {
        let c = conn();
        let kept = store(&c, &"a".repeat(40_000)).unwrap();
        let dropped = store(&c, &"b".repeat(40_000)).unwrap();
        c.execute(
            "INSERT INTO request_bodies (log_id, request_layout) VALUES (1, ?1), (2, ?2)",
            params![&kept, &dropped],
        )
        .unwrap();

        c.execute("DELETE FROM request_bodies WHERE log_id = 2", [])
            .unwrap();
        let removed = sweep(&c).unwrap();
        assert!(removed > 0, "the dropped body's chunks should go");

        let mut r = BodyReader::new(&c);
        assert_eq!(r.read(&kept).unwrap(), "a".repeat(40_000));
    }

    #[test]
    fn sweeping_twice_removes_nothing_the_second_time() {
        let c = conn();
        let layout = store(&c, &"c".repeat(40_000)).unwrap();
        c.execute(
            "INSERT INTO request_bodies (log_id, request_layout) VALUES (1, ?1)",
            params![&layout],
        )
        .unwrap();
        sweep(&c).unwrap();
        assert_eq!(sweep(&c).unwrap(), 0);
    }

    #[test]
    fn chunk_boundaries_are_deterministic() {
        let text = "the same bytes must chunk the same way every time".repeat(500);
        let a: Vec<usize> = split(text.as_bytes()).iter().map(|c| c.len()).collect();
        let b: Vec<usize> = split(text.as_bytes()).iter().map(|c| c.len()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn chunks_respect_the_minimum_and_maximum() {
        // A run of one byte value has no natural cut points; the maximum has to
        // close it rather than let it grow without bound.
        for chunk in split(&vec![0u8; CHUNK_MAX * 3]) {
            assert!(chunk.len() >= CHUNK_MIN);
            assert!(chunk.len() <= CHUNK_MAX);
        }
    }
}
