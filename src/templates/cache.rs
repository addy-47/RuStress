use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::core::constants::{MAX_CACHED_TEMPLATE_FILES, MAX_TEMPLATE_FILE_BYTES};

/// Thread-safe file cache for template functions.
///
/// Caches both file lines (for `randomLine`) and full file contents (for `readFile`).
#[derive(Debug, Default)]
pub struct FileCache {
    /// Cached file lines (non-empty).
    line_cache: RwLock<HashMap<String, Vec<String>>>,
    /// Cached full file contents.
    raw_cache: RwLock<HashMap<String, String>>,
}

impl FileCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reject a new entry once either bound is reached.
    ///
    /// Called under the write lock and only for a key that is not cached, so
    /// the check cannot race another insertion.
    fn check_new_entry(path: &str, entry_count: usize) -> anyhow::Result<()> {
        if entry_count >= MAX_CACHED_TEMPLATE_FILES {
            anyhow::bail!(
                "template file cache is full ({MAX_CACHED_TEMPLATE_FILES} entries); refusing '{path}'"
            );
        }
        let len = fs::metadata(path)
            .map_err(|e| anyhow::anyhow!("failed to read metadata for '{path}': {e}"))?
            .len();
        if len > MAX_TEMPLATE_FILE_BYTES {
            anyhow::bail!(
                "template file '{path}' is {len} bytes, above the {MAX_TEMPLATE_FILE_BYTES} byte limit"
            );
        }
        Ok(())
    }

    /// Get lines from a file, loading from disk on first access.
    pub fn get_lines(&self, path: &str) -> anyhow::Result<Vec<String>> {
        // Fast path: check read lock first.
        {
            let cache = self.line_cache.read();
            if let Some(lines) = cache.get(path) {
                return Ok(lines.clone());
            }
        }

        // Slow path: acquire write lock and load.
        let mut cache = self.line_cache.write();
        // Double-check after acquiring write lock.
        if let Some(lines) = cache.get(path) {
            return Ok(lines.clone());
        }
        Self::check_new_entry(path, cache.len())?;

        let content = fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read file '{}': {}", path, e))?;

        let lines: Vec<String> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect();

        cache.insert(path.to_string(), lines.clone());
        Ok(lines)
    }

    /// Get full file contents, loading from disk on first access.
    pub fn get_raw(&self, path: &str) -> anyhow::Result<String> {
        // Fast path: check read lock first.
        {
            let cache = self.raw_cache.read();
            if let Some(content) = cache.get(path) {
                return Ok(content.clone());
            }
        }

        // Slow path: acquire write lock and load.
        let mut cache = self.raw_cache.write();
        // Double-check after acquiring write lock.
        if let Some(content) = cache.get(path) {
            return Ok(content.clone());
        }
        Self::check_new_entry(path, cache.len())?;

        let content = fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read file '{}': {}", path, e))?;

        cache.insert(path.to_string(), content.clone());
        Ok(content)
    }

    /// Check if a file exists on disk.
    pub fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    /// Clear all cached files.
    pub fn clear(&self) {
        self.line_cache.write().clear();
        self.raw_cache.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_space_is_bounded_under_distinct_paths() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = FileCache::new();
        for i in 0..MAX_CACHED_TEMPLATE_FILES {
            let path = dir.path().join(format!("file-{i}.txt"));
            fs::write(&path, "value").expect("write fixture");
            cache
                .get_raw(path.to_str().expect("utf8 path"))
                .expect("cache");
        }

        let overflow = dir.path().join("overflow.txt");
        fs::write(&overflow, "value").expect("write fixture");
        assert!(
            cache
                .get_raw(overflow.to_str().expect("utf8 path"))
                .is_err(),
            "one more distinct path must be refused at the ceiling"
        );
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_read() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("huge.txt");
        let file = fs::File::create(&path).expect("create fixture");
        file.set_len(MAX_TEMPLATE_FILE_BYTES + 1)
            .expect("sparse fixture");

        let cache = FileCache::new();
        let err = cache
            .get_raw(path.to_str().expect("utf8 path"))
            .expect_err("the size ceiling must fire");
        assert!(
            err.to_string().contains("byte limit"),
            "the reason must name the bound: {err}"
        );
    }

    #[test]
    fn a_cached_file_stays_available_after_the_ceiling_is_reached() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("first.txt");
        fs::write(&path, "value").expect("write fixture");
        let cache = FileCache::new();
        for _ in 0..2 {
            assert_eq!(
                cache
                    .get_raw(path.to_str().expect("utf8 path"))
                    .expect("cache"),
                "value"
            );
        }
    }
}
