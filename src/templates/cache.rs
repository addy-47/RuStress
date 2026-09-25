use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

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
