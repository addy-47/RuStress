pub mod cache;
pub mod context;
pub mod engine;

pub use cache::FileCache;
pub use context::TemplateContext;
pub use engine::TemplateEngine;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_preprocess_userid() {
        let engine = TemplateEngine::new();
        let tpl = engine.parse("test", "Hello {{userID}}!").unwrap();
        let ctx = TemplateContext::new("user-42".into(), "abc".into());
        let result = engine.execute(&tpl, &ctx).unwrap();
        assert_eq!(result, "Hello user-42!");
    }

    #[test]
    fn test_preprocess_uuid() {
        let engine = TemplateEngine::new();
        let tpl = engine.parse("test", "ID: {{uuid}}").unwrap();
        let ctx = TemplateContext::new("user-1".into(), "req-123".into());
        let result = engine.execute(&tpl, &ctx).unwrap();
        // Should generate a new UUID, not "req-123"
        assert!(result.starts_with("ID: "));
        assert!(result.len() > 10);
    }

    #[test]
    fn test_random_int_range() {
        let engine = TemplateEngine::new();
        let tpl = engine.parse("test", "{{ random_int(10, 20) }}").unwrap();
        let ctx = TemplateContext::default();

        // Run multiple times to verify randomness
        for _ in 0..50 {
            let result = engine.execute(&tpl, &ctx).unwrap();
            let val: i64 = result.parse().unwrap();
            assert!(val >= 10 && val < 20, "value {} not in [10, 20)", val);
        }
    }

    #[test]
    fn test_random_choice() {
        let engine = TemplateEngine::new();
        let tpl = engine.parse("test", r#"{{ random_choice(["A", "B", "C"]) }}"#).unwrap();
        let ctx = TemplateContext::default();

        let mut results = std::collections::HashSet::new();
        for _ in 0..100 {
            let result = engine.execute(&tpl, &ctx).unwrap();
            results.insert(result);
        }
        // Should have seen at least 2 different values (probabilistic)
        assert!(results.len() >= 2, "random_choice not diverse: {:?}", results);
    }

    #[test]
    fn test_read_file() {
        let tmp = NamedTempFile::new().unwrap();
        fs::write(tmp.path(), "hello world").unwrap();

        let engine = TemplateEngine::new();
        let ctx = TemplateContext::default();

        // Use execute_str since the path is in the template
        let result = engine.execute_str(
            &format!(r#"{{{{ read_file("{}") }}}}"#, tmp.path().display()),
            &ctx,
        ).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_random_line() {
        let tmp = NamedTempFile::new().unwrap();
        fs::write(tmp.path(), "line1\nline2\nline3\n").unwrap();

        let engine = TemplateEngine::new();
        let ctx = TemplateContext::default();

        let mut results = std::collections::HashSet::new();
        for _ in 0..50 {
            let result = engine.execute_str(
                &format!(r#"{{{{ random_line("{}") }}}}"#, tmp.path().display()),
                &ctx,
            ).unwrap();
            results.insert(result);
        }
        // Should have seen at least 2 different lines
        assert!(results.len() >= 2, "random_line not diverse: {:?}", results);
    }

    #[test]
    fn test_combined_template() {
        let engine = TemplateEngine::new();
        let tpl = engine.parse(
            "combined",
            "User={{userID}}, Rand={{ random_int(1, 100) }}, UUID={{uuid()}}",
        ).unwrap();
        let ctx = TemplateContext::new("test-user".into(), "req-uuid".into());

        let result = engine.execute(&tpl, &ctx).unwrap();
        assert!(result.contains("User=test-user"));
        assert!(result.contains("Rand="));
        assert!(result.contains("UUID="));
    }

    #[test]
    fn test_parse_invalid_template() {
        let engine = TemplateEngine::new();
        let result = engine.parse("bad", "{{ invalid syntax }}}");
        assert!(result.is_err());
    }

    #[test]
    fn test_file_cache_caching() {
        let tmp = NamedTempFile::new().unwrap();
        fs::write(tmp.path(), "cached content").unwrap();

        let engine = TemplateEngine::new();
        let path = tmp.path().to_str().unwrap();

        // First read loads from disk
        let content1 = engine.file_cache().get_raw(path).unwrap();
        // Second read should hit cache (modify file after cache)
        fs::write(tmp.path(), "modified content").unwrap();
        let content2 = engine.file_cache().get_raw(path).unwrap();

        // Cache should return original content, not modified
        assert_eq!(content1, "cached content");
        assert_eq!(content2, "cached content");
    }

    #[test]
    fn test_file_cache_clear() {
        let tmp = NamedTempFile::new().unwrap();
        fs::write(tmp.path(), "data").unwrap();

        let engine = TemplateEngine::new();
        let path = tmp.path().to_str().unwrap();

        engine.file_cache().get_raw(path).unwrap();
        engine.file_cache().clear();

        // After clear, modifying the file should show on next read
        fs::write(tmp.path(), "new data").unwrap();
        let content = engine.file_cache().get_raw(path).unwrap();
        assert_eq!(content, "new data");
    }
}
