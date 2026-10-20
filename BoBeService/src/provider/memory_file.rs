use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::error::AppError;

pub(crate) const DEFAULT_BODY: &str =
    "# BoBe Memory\n\n## Profile\n\n## Active Goals\n\n## Long-term\n\n## Recent\n";

pub(crate) const TARGET_MAX_BYTES: usize = 50 * 1024;
const RECENT_MAX_BYTES: usize = 8 * 1024;

pub(crate) struct MemoryFile {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
    revision: AtomicU64,
}

pub(crate) struct WriterGuard<'a> {
    file: &'a MemoryFile,
    _guard: OwnedMutexGuard<()>,
}

impl WriterGuard<'_> {
    pub(crate) async fn read(&self) -> Result<String, AppError> {
        self.file.read_unlocked().await
    }
    pub(crate) async fn replace_all(&self, body: String) -> Result<(), AppError> {
        self.file.commit(&body).await
    }
}

impl MemoryFile {
    pub(crate) fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            write_lock: Arc::new(Mutex::new(())),
            revision: AtomicU64::new(1),
        })
    }

    pub(crate) async fn acquire_writer(&self) -> WriterGuard<'_> {
        let guard = Arc::clone(&self.write_lock).lock_owned().await;
        WriterGuard {
            file: self,
            _guard: guard,
        }
    }

    pub(crate) async fn read(&self) -> Result<String, AppError> {
        self.read_unlocked().await
    }

    pub(crate) async fn context(&self, max_bytes: usize) -> Result<String, AppError> {
        Ok(context_excerpt(
            &bound_recent(&self.read().await?),
            max_bytes,
        ))
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    async fn read_unlocked(&self) -> Result<String, AppError> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DEFAULT_BODY.to_string()),
            Err(e) => Err(AppError::Io(e)),
        }
    }

    pub(crate) async fn append_under(&self, section: &str, entry: &str) -> Result<(), AppError> {
        let entry = entry.trim();
        Self::validate_edit_line("appended", entry)?;
        let _guard = self.write_lock.lock().await;
        let body = self.read_unlocked().await?;
        if section_range(&body, section).is_some_and(|range| {
            body[range].lines().any(|line| {
                line.split_once(" — ")
                    .is_some_and(|(_, text)| text == entry)
            })
        }) {
            return Ok(());
        }
        let now = Utc::now();
        let date = now.format("%Y-%m-%d %H:%M");
        let line = format!("- {date} — {entry}");
        let new_body = upsert_bullet(&body, section, &line);
        self.commit(&bound_recent(&new_body)).await
    }

    pub(crate) async fn replace_all(&self, body: String) -> Result<(), AppError> {
        let _guard = self.write_lock.lock().await;
        self.commit(&body).await
    }

    pub(crate) async fn replace_exact_line(
        &self,
        expected_revision: u64,
        current_line: &str,
        replacement_line: &str,
    ) -> Result<(), AppError> {
        Self::validate_edit_line("current", current_line)?;
        Self::validate_edit_line("replacement", replacement_line)?;
        let _guard = self.write_lock.lock().await;
        self.ensure_revision(expected_revision)?;
        let body = self.read_unlocked().await?;
        let updated = Self::edit_exact_line(&body, current_line, Some(replacement_line))?;
        self.commit(&updated).await
    }

    pub(crate) async fn remove_exact_line(
        &self,
        expected_revision: u64,
        current_line: &str,
    ) -> Result<(), AppError> {
        Self::validate_edit_line("current", current_line)?;
        let _guard = self.write_lock.lock().await;
        self.ensure_revision(expected_revision)?;
        let body = self.read_unlocked().await?;
        let updated = Self::edit_exact_line(&body, current_line, None)?;
        self.commit(&updated).await
    }

    fn ensure_revision(&self, expected_revision: u64) -> Result<(), AppError> {
        let current = self.revision();
        if current == expected_revision {
            Ok(())
        } else {
            Err(AppError::Conflict(format!(
                "memory changed since it was read: expected revision {expected_revision}, current revision {current}"
            )))
        }
    }

    async fn commit(&self, body: &str) -> Result<(), AppError> {
        if body.len() > TARGET_MAX_BYTES {
            return Err(AppError::Validation(format!(
                "Memory must be at most {TARGET_MAX_BYTES} bytes; consolidate or shorten it before adding more durable facts"
            )));
        }
        commit(&self.path, body).await?;
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn validate_edit_line(label: &str, line: &str) -> Result<(), AppError> {
        if line.is_empty() || line.contains(['\r', '\n']) {
            Err(AppError::Validation(format!(
                "{label} memory line must be one non-empty line"
            )))
        } else {
            Ok(())
        }
    }

    fn edit_exact_line(
        body: &str,
        current_line: &str,
        replacement_line: Option<&str>,
    ) -> Result<String, AppError> {
        let matches = body.lines().filter(|line| *line == current_line).count();
        if matches != 1 {
            return Err(AppError::Conflict(format!(
                "memory edit requires exactly one matching line; found {matches}"
            )));
        }
        let mut lines = Vec::new();
        for line in body.lines() {
            if line == current_line {
                if let Some(replacement) = replacement_line {
                    lines.push(replacement);
                }
            } else {
                lines.push(line);
            }
        }
        let mut output = lines.join("\n");
        if body.ends_with('\n') {
            output.push('\n');
        }
        Ok(output)
    }
}

fn section_range(body: &str, heading: &str) -> Option<std::ops::Range<usize>> {
    let header = format!("## {heading}");
    let mut offset = 0;
    let mut start = None;
    for line in body.split_inclusive('\n') {
        let title = line.trim_end();
        if let Some(start) = start
            && title.starts_with("## ")
        {
            return Some(start..offset);
        }
        if title == header {
            start = Some(offset);
        }
        offset += line.len();
    }
    start.map(|start| start..body.len())
}

fn bound_recent(body: &str) -> String {
    let Some(range) = section_range(body, "Recent") else {
        return body.to_owned();
    };
    let available = TARGET_MAX_BYTES
        .saturating_sub(body.len() - range.len())
        .min(RECENT_MAX_BYTES);
    let header = "## Recent\n";
    let mut remaining = available.saturating_sub(header.len());
    let mut lines = Vec::new();
    let content_start = body[range.clone()]
        .find('\n')
        .map_or(range.end, |offset| range.start + offset + 1);
    for line in body[content_start..range.end].lines().rev() {
        if line.len() + 1 > remaining {
            break;
        }
        remaining -= line.len() + 1;
        lines.push(line);
    }
    lines.reverse();
    format!(
        "{}{}{}\n{}",
        &body[..range.start],
        header,
        lines.join("\n"),
        &body[range.end..]
    )
}

fn context_excerpt(body: &str, max_bytes: usize) -> String {
    if body.len() <= max_bytes {
        return body.to_owned();
    }
    const NOTICE: &str = "\n[Memory excerpt truncated; use bobe_memory_read for omitted facts.]\n";
    let end = body.floor_char_boundary(max_bytes.saturating_sub(NOTICE.len()));
    format!("{}{}", &body[..end], NOTICE)
}

fn upsert_bullet(body: &str, heading: &str, line: &str) -> String {
    let header = format!("## {heading}");
    let Some(range) = section_range(body, heading) else {
        let mut out = body.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&header);
        out.push_str("\n\n");
        out.push_str(line);
        out.push('\n');
        return out;
    };
    let next_section = range.end;

    let (head, rest) = body.split_at(next_section);
    let head_trimmed = head.trim_end();
    let mut out = head_trimmed.to_string();
    out.push('\n');
    out.push_str(line);
    out.push('\n');
    if !rest.is_empty() {
        if !rest.starts_with('\n') {
            out.push('\n');
        }
        out.push_str(rest);
    }
    out
}

async fn commit(path: &Path, body: &str) -> Result<(), AppError> {
    crate::util::durable_fs::atomic_write(path, body.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bbm-{}", Uuid::new_v4().as_simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn appended_entries_cannot_create_sections() {
        let dir = tempdir();
        let memory = MemoryFile::new(dir.join("memory.md"));
        for entry in [
            "Working\n## Long-term\n- false fact",
            "Working\r## Profile",
            "  \n ",
        ] {
            assert!(memory.append_under("Recent", entry).await.is_err());
        }
        assert_eq!(memory.revision(), 1);
        assert_eq!(memory.read().await.unwrap(), DEFAULT_BODY);
        memory
            .append_under("Recent", "  Working on the release\n")
            .await
            .unwrap();
        let body = memory.read().await.unwrap();
        assert_eq!(body.matches("## ").count(), 4);
        assert!(body.ends_with(" — Working on the release\n"));
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn a_recent_fact_can_be_promoted_to_durable_memory() {
        let dir = tempdir();
        let memory = MemoryFile::new(dir.join("memory.md"));
        memory
            .append_under("Recent", "User likes tea")
            .await
            .unwrap();
        memory
            .append_under("Long-term", "User likes tea")
            .await
            .unwrap();
        let body = memory.read().await.unwrap();
        assert!(body[section_range(&body, "Long-term").unwrap()].contains("User likes tea"));
        assert_eq!(body.matches("User likes tea").count(), 2);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn oversized_write_preserves_existing_memory() {
        let dir = tempdir();
        let memory = MemoryFile::new(dir.join("memory.md"));
        memory.replace_all("Durable fact".into()).await.unwrap();
        assert!(
            memory
                .replace_all("x".repeat(TARGET_MAX_BYTES + 1))
                .await
                .is_err()
        );
        assert_eq!(memory.read().await.unwrap(), "Durable fact");
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn recent_observations_are_bounded_and_duplicates_do_not_refresh_context() {
        let dir = tempdir();
        let memory = MemoryFile::new(dir.join("memory.md"));
        memory
            .append_under("Long-term", "Keep this durable fact")
            .await
            .unwrap();
        for index in 0..80 {
            memory
                .append_under(
                    "Recent",
                    &format!("observation {index}: {}", "x".repeat(180)),
                )
                .await
                .unwrap();
        }
        let before = memory.read().await.unwrap();
        assert!(before.contains("Keep this durable fact"));
        assert!(!before.contains("observation 0:"));
        assert!(before.contains("observation 79:"));
        assert!(section_range(&before, "Recent").unwrap().len() <= RECENT_MAX_BYTES);
        let revision = memory.revision();
        memory
            .append_under("Recent", &format!("observation 79: {}", "x".repeat(180)))
            .await
            .unwrap();
        assert_eq!(memory.revision(), revision);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn oversized_memory_is_excerpted_without_rewriting_user_data() {
        let dir = tempdir();
        let path = dir.join("memory.md");
        let body = "🦀".repeat(40_000);
        tokio::fs::write(&path, &body).await.unwrap();
        let memory = MemoryFile::new(path);
        let excerpt = memory.context(4096).await.unwrap();
        assert!(excerpt.len() <= 4096);
        assert!(excerpt.contains("truncated"));
        assert_eq!(memory.read().await.unwrap(), body);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn read_returns_default_when_missing() {
        let dir = tempdir();
        let mem = MemoryFile::new(dir.join("memory.md"));
        let body = mem.read().await.unwrap();
        assert!(body.contains("# BoBe Memory"));
        assert!(body.contains("## Recent"));
    }

    #[tokio::test]
    async fn append_under_existing_section() {
        let dir = tempdir();
        let mem = MemoryFile::new(dir.join("memory.md"));
        mem.append_under("Recent", "User asked about Foo")
            .await
            .unwrap();
        mem.append_under("Recent", "User asked about Bar")
            .await
            .unwrap();
        assert_eq!(mem.revision(), 3);
        let body = mem.read().await.unwrap();

        assert!(body.contains("User asked about Foo"));
        assert!(body.contains("User asked about Bar"));
        assert!(
            body.matches("## Recent").count() == 1,
            "no duplicate sections"
        );
    }

    #[tokio::test]
    async fn append_creates_missing_section() {
        let dir = tempdir();
        let mem = MemoryFile::new(dir.join("memory.md"));
        mem.append_under("BrandNew", "thing").await.unwrap();
        let body = mem.read().await.unwrap();
        assert!(body.contains("## BrandNew"));
        assert!(body.contains("- ") && body.contains("thing"));
    }

    #[tokio::test]
    async fn writer_guard_replace_all_overwrites() {
        let dir = tempdir();
        let mem = MemoryFile::new(dir.join("memory.md"));
        mem.append_under("Recent", "first").await.unwrap();
        let writer = mem.acquire_writer().await;
        writer.replace_all("# fresh\n".to_string()).await.unwrap();
        drop(writer);
        assert_eq!(mem.read().await.unwrap(), "# fresh\n");
    }

    #[tokio::test]
    async fn concurrent_appends_are_serialized() {
        let dir = tempdir();
        let mem = MemoryFile::new(dir.join("memory.md"));
        let mut handles = Vec::new();
        for i in 0..10 {
            let m = Arc::clone(&mem);
            handles.push(tokio::spawn(async move {
                m.append_under("Recent", &format!("entry {i}")).await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }
        let body = mem.read().await.unwrap();
        for i in 0..10 {
            assert!(body.contains(&format!("entry {i}")), "missing entry {i}");
        }
    }

    #[tokio::test]
    async fn exact_edits_require_current_revision_and_unique_line() {
        let dir = tempdir();
        let mem = MemoryFile::new(dir.join("memory.md"));
        mem.replace_all("# Memory\n\n- old\n".into()).await.unwrap();
        let revision = mem.revision();
        mem.replace_exact_line(revision, "- old", "- new")
            .await
            .unwrap();
        assert!(mem.read().await.unwrap().contains("- new"));
        assert!(
            mem.remove_exact_line(revision, "- new").await.is_err(),
            "stale revisions must fail"
        );
        let revision = mem.revision();
        mem.remove_exact_line(revision, "- new").await.unwrap();
        assert!(!mem.read().await.unwrap().contains("- new"));
    }

    #[tokio::test]
    async fn exact_edit_normalizes_crlf_without_false_success() {
        let dir = tempdir();
        let mem = MemoryFile::new(dir.join("memory.md"));
        mem.replace_all("# Memory\r\n\r\n- old\r\n".into())
            .await
            .unwrap();
        let revision = mem.revision();

        mem.replace_exact_line(revision, "- old", "- new")
            .await
            .unwrap();

        assert_eq!(mem.read().await.unwrap(), "# Memory\n\n- new\n");
    }
}
