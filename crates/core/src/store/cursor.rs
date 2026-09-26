//! Incremental sync cursors (spec §5 `sync_cursors`, design proven by cc-switch).
//!
//! A cursor is `last_byte_offset` + sha2-256 fingerprint of the 256 bytes ending
//! at that offset. On each scan:
//!   - file grew AND tail segment still matches → read only `[offset, len)`
//!   - file shrank OR fingerprint mismatch → the file was truncated/rotated;
//!     pin the cursor to EOF and NEVER replay (replaying rolled-up bytes would
//!     double-count forever).
//!   - no cursor yet → full read from 0, then store cursor at EOF.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Bytes hashed for the tail fingerprint.
const TAIL_LEN: u64 = 256;

#[derive(Debug, Clone, Default)]
pub struct FileCursor {
    pub offset: u64,
    pub fingerprint: Option<String>,
    /// Opaque per-adapter resume state (e.g. Codex's cumulative counters).
    pub state: Option<String>,
}

/// What the caller must do for this file on this scan.
#[derive(Debug, PartialEq, Eq)]
pub enum CursorAction {
    /// Append-safe: parse bytes `[from, file_len)`.
    Append { from: u64 },
    /// Never seen (or migrated) — parse the whole file.
    Full,
    /// File changed incompatibly (truncated/rotated/rewritten). Pin to EOF,
    /// skip parsing. Carries the file length the cursor was pinned to.
    SkipPinned { eof: u64 },
    /// Nothing new (size unchanged or smaller-but-identical tail… rare).
    Unchanged,
}

/// sha2-256 hex of the bytes in `[offset - TAIL_LEN, offset)` (or the whole
/// prefix when shorter than TAIL_LEN). Returns `None` for offset == 0.
pub fn tail_fingerprint(path: &Path, offset: u64) -> Result<Option<String>> {
    if offset == 0 {
        return Ok(None);
    }
    let mut f = std::fs::File::open(path)
        .with_context(|| format!("open {} for fingerprint", path.display()))?;
    let start = offset.saturating_sub(TAIL_LEN);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; (offset - start) as usize];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    let mut h = Sha256::new();
    h.update(&buf);
    let digest = h.finalize();
    Ok(Some(digest.iter().map(|b| format!("{b:02x}")).collect()))
}

impl super::Store {
    pub fn load_cursor(&self, file_path: &str) -> Result<FileCursor> {
        let row = self.conn().query_row(
            "SELECT last_byte_offset, last_tail_fingerprint, adapter_state FROM sync_cursors WHERE file_path=?1",
            params![file_path],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        );
        match row.optional()? {
            Some((off, fp, state)) => Ok(FileCursor {
                offset: off.max(0) as u64,
                fingerprint: fp,
                state,
            }),
            None => Ok(FileCursor::default()),
        }
    }

    /// Decide how to scan `path` this round. `len`/`mtime_ms` are the caller's
    /// `fs::metadata` (kept outside so tests can fabricate them).
    pub fn cursor_action(&self, path: &Path, file_key: &str, len: u64) -> Result<CursorAction> {
        let cur = self.load_cursor(file_key)?;
        if cur.offset == 0 && cur.fingerprint.is_none() {
            return Ok(if len == 0 {
                CursorAction::Unchanged
            } else {
                CursorAction::Full
            });
        }
        if len < cur.offset {
            // Truncated/rotated — pin, never replay.
            return Ok(CursorAction::SkipPinned { eof: len });
        }
        if len == cur.offset {
            return Ok(CursorAction::Unchanged);
        }
        // File grew — verify the bytes preceding the cursor are unchanged.
        let fp = tail_fingerprint(path, cur.offset)?;
        if fp == cur.fingerprint {
            Ok(CursorAction::Append { from: cur.offset })
        } else {
            Ok(CursorAction::SkipPinned { eof: len })
        }
    }

    /// Persist a cursor after a successful parse (`offset` = new EOF position
    /// actually consumed; callers pass file len after read).
    pub fn save_cursor(
        &self,
        source: &str,
        file_key: &str,
        path: &Path,
        offset: u64,
        mtime_ms: i64,
        state: Option<&str>,
    ) -> Result<()> {
        let fp = tail_fingerprint(path, offset)?;
        self.conn().execute(
            "INSERT INTO sync_cursors(source, file_path, last_byte_offset, last_tail_fingerprint, last_modified, last_synced_at, adapter_state)
             VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(file_path) DO UPDATE SET
               source=excluded.source, last_byte_offset=excluded.last_byte_offset,
               last_tail_fingerprint=excluded.last_tail_fingerprint,
               last_modified=excluded.last_modified, last_synced_at=excluded.last_synced_at,
               adapter_state=excluded.adapter_state",
            params![source, file_key, offset as i64, fp, mtime_ms, super::now_ms(), state],
        )?;
        Ok(())
    }

    /// Pin a cursor to EOF without parsing (SkipPinned path) — keeps prior
    /// adapter_state (we may still need it if the file regrows legitimately).
    pub fn pin_cursor_eof(
        &self,
        source: &str,
        file_key: &str,
        path: &Path,
        eof: u64,
        mtime_ms: i64,
    ) -> Result<()> {
        let fp = tail_fingerprint(path, eof)?;
        self.conn().execute(
            "INSERT INTO sync_cursors(source, file_path, last_byte_offset, last_tail_fingerprint, last_modified, last_synced_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(file_path) DO UPDATE SET
               source=excluded.source, last_byte_offset=excluded.last_byte_offset,
               last_tail_fingerprint=excluded.last_tail_fingerprint,
               last_modified=excluded.last_modified, last_synced_at=excluded.last_synced_at",
            params![source, file_key, eof as i64, fp, mtime_ms, super::now_ms()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::io::Write;

    fn tmpfile(content: &[u8]) -> (TempfilePathGuard, std::path::PathBuf) {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "cl-cursor-{}-{}.log",
            std::process::id(),
            content.len()
        ));
        std::fs::write(&p, content).unwrap();
        (TempfilePathGuard(p.clone()), p)
    }

    // Ensures cleanup even if assertions panic.
    struct TempfilePathGuard(std::path::PathBuf);
    impl Drop for TempfilePathGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn full_then_append_then_pin_on_truncate() -> Result<()> {
        let s = Store::open_memory()?;
        let (_g, p) = tmpfile(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let key = p.to_string_lossy().to_string();

        // First scan: Full.
        assert_eq!(s.cursor_action(&p, &key, 40)?, CursorAction::Full);
        s.save_cursor("test", &key, &p, 40, 0)?;

        // Unchanged.
        assert_eq!(s.cursor_action(&p, &key, 40)?, CursorAction::Unchanged);

        // Append more bytes → Append{40}.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p)?;
        f.write_all(b"bbbbbbbbbb")?;
        drop(f);
        assert_eq!(
            s.cursor_action(&p, &key, 50)?,
            CursorAction::Append { from: 40 }
        );
        s.save_cursor("test", &key, &p, 50, 0)?;

        // Truncate & rewrite → pin to EOF, never replay.
        std::fs::write(&p, b"cc")?;
        assert_eq!(
            s.cursor_action(&p, &key, 2)?,
            CursorAction::SkipPinned { eof: 2 }
        );
        Ok(())
    }

    #[test]
    fn rewrite_same_growth_prefix_is_detected() -> Result<()> {
        // File "grew" but the bytes before the old offset changed → pin.
        let s = Store::open_memory()?;
        let (_g, p) = tmpfile(b"0123456789abcdef0123456789abcdef0123456789");
        let key = p.to_string_lossy().to_string();
        s.save_cursor("test", &key, &p, 40, 0)?;
        // Rewrite first bytes, then extend — naive offset check would replay.
        let mut v = std::fs::read(&p)?;
        v[0] = b'X';
        v.extend_from_slice(b"newtail");
        std::fs::write(&p, &v)?;
        assert!(matches!(
            s.cursor_action(&p, &key, v.len() as u64)?,
            CursorAction::SkipPinned { .. }
        ));
        Ok(())
    }
}
