use reqwest::header::HeaderName;
use reqwest::{Client, RequestBuilder};

use crate::core::config::Config;

use crate::templates::{TemplateContext, TemplateEngine};

/// A single request target decomposed into its static and templated parts.
///
/// Everything that can be decided once — parsing the HTTP method, lowering and
/// validating header names, detecting an existing `content-type`, classifying
/// `@file` bodies — is decided at construction time. Per-request work is then
/// limited to rendering the parts that actually contain a template directive
/// and handing them to `reqwest`.
///
/// This is the difference between a load generator that can saturate a NIC and
/// one that spends its budget re-parsing templates and re-allocating header
/// keys.
#[derive(Clone, Debug)]
pub struct PreparedRequest {
    method: reqwest::Method,
    static_url: String,
    url_template: Option<String>,
    static_body: Option<String>,
    static_headers: Vec<(reqwest::header::HeaderName, String)>,
    templated_headers: Vec<(reqwest::header::HeaderName, String)>,
    inject_content_type: bool,
    /// Label recorded on every result produced by this request.
    pub label: String,
}
impl PreparedRequest {
    /// Decompose a validated config into a reusable request plan.
    ///
    /// An `@file` body is read here rather than per request. The file is
    /// static for the duration of a run, so reading it once removes both a
    /// per-request file lookup and the hazard of splicing an arbitrary path
    /// into a template literal. A missing file is reported at construction
    /// instead of silently producing empty bodies mid-run.
    pub fn new(cfg: &Config, engine: &TemplateEngine) -> anyhow::Result<Self> {
        let method = reqwest::Method::from_bytes(cfg.method.as_bytes())
            .unwrap_or(reqwest::Method::GET);

        let url_template = has_template(&cfg.url).then(|| cfg.url.clone());

        let static_body = match cfg.body.as_deref() {
            None | Some("") => None,
            Some(body) if body.starts_with('@') => {
                let path = body.strip_prefix('@').unwrap_or(body);
                let contents = engine
                    .file_cache()
                    .get_raw(path)
                    .map_err(|e| anyhow::anyhow!("failed to load body file '{path}': {e}"))?;
                Some(contents)
            }
            Some(body) if has_template(body) => {
                return Err(anyhow::anyhow!(
                    "templated inline bodies are not supported; use @file with a template inside it"
                ));
            }
            Some(body) => Some(body.to_string()),
        };

        let mut has_content_type = false;
        let mut static_headers = Vec::with_capacity(cfg.headers.len());
        let mut templated_headers = Vec::new();

        for (name, value) in &cfg.headers {
            let header_name = match HeaderName::from_bytes(name.as_bytes()) {
                Ok(parsed) => parsed,
                Err(_) => continue,
            };
            if header_name == reqwest::header::CONTENT_TYPE {
                has_content_type = true;
            }
            if has_template(value) {
                templated_headers.push((header_name, value.clone()));
            } else {
                static_headers.push((header_name, value.clone()));
            }
        }

        let carries_body = static_body.is_some();

        Ok(Self {
            method,
            static_url: cfg.url.clone(),
            url_template,
            static_body,
            static_headers,
            templated_headers,
            inject_content_type: carries_body && !has_content_type,
            label: "custom".to_string(),
        })
    }

    /// Whether any part of this request requires per-request rendering.
    pub fn is_templated(&self) -> bool {
        self.url_template.is_some() || !self.templated_headers.is_empty()
    }

    /// Render the templated parts and produce a `reqwest` request builder.
    ///
    /// `Err` carries a human-readable reason; a failure to render a template
    /// must not abort the run, because a load test against a broken target is
    /// itself a valid result.
    pub fn build(
        &self,
        client: &Client,
        engine: &TemplateEngine,
        ctx: &TemplateContext,
    ) -> Result<RequestBuilder, String> {
        let url = match &self.url_template {
            Some(tpl) => render(engine, tpl, ctx)?,
            None => self.static_url.clone(),
        };
        if url.is_empty() {
            return Err("request url is empty".to_string());
        }

        let mut request = client.request(self.method.clone(), &url);

        for (name, value) in &self.static_headers {
            request = request.header(name, value);
        }
        for (name, template) in &self.templated_headers {
            request = request.header(name, render(engine, template, ctx)?);
        }

        if let Some(body) = &self.static_body {
            request = request.body(body.clone());
        }

        if self.inject_content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
        }

        Ok(request)
    }
}

/// Whether a string contains a template directive.
fn has_template(value: &str) -> bool {
    value.contains("{{")
}

/// Render a template, mapping failures onto an error string.
fn render(engine: &TemplateEngine, template: &str, ctx: &TemplateContext) -> Result<String, String> {
    engine
        .execute_str(template, ctx)
        .map_err(|e| format!("template render failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use std::io::Write;

    fn engine() -> TemplateEngine {
        TemplateEngine::new()
    }

    fn cfg_with(headers: IndexMap<String, String>, body: Option<&str>) -> Config {
        Config {
            url: "http://127.0.0.1:9/x".into(),
            headers,
            body: body.map(str::to_string),
            steady_dur_secs: 1,
            ..Default::default()
        }
    }

    fn prepared(cfg: &Config) -> PreparedRequest {
        PreparedRequest::new(cfg, &engine()).expect("config should decompose")
    }

    #[test]
    fn static_request_needs_no_rendering() {
        assert!(!prepared(&cfg_with(IndexMap::new(), None)).is_templated());
    }

    #[test]
    fn templated_url_is_detected() {
        let mut cfg = cfg_with(IndexMap::new(), None);
        cfg.url = "http://127.0.0.1:9/{{ uuid() }}".into();
        assert!(prepared(&cfg).is_templated());
    }

    #[test]
    fn templated_header_is_detected() {
        let mut headers = IndexMap::new();
        headers.insert("X-Id".to_string(), "{{ uuid() }}".to_string());
        assert!(prepared(&cfg_with(headers, None)).is_templated());
    }

    #[test]
    fn explicit_content_type_suppresses_injection() {
        let mut headers = IndexMap::new();
        headers.insert("content-type".to_string(), "text/plain".to_string());
        let p = prepared(&cfg_with(headers, Some("hello")));
        assert!(!p.inject_content_type);
    }

    #[test]
    fn body_without_content_type_injects_json() {
        assert!(prepared(&cfg_with(IndexMap::new(), Some("hello"))).inject_content_type);
    }

    #[test]
    fn no_body_means_no_content_type_injection() {
        assert!(!prepared(&cfg_with(IndexMap::new(), None)).inject_content_type);
    }

    #[test]
    fn at_prefixed_body_is_read_from_disk() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{{\"id\":\"{{{{ uuid() }}}}\"}}").unwrap();

        let path = format!("@{}", file.path().display());
        let cfg = cfg_with(IndexMap::new(), Some(&path));
        let p = prepared(&cfg);

        assert_eq!(
            p.static_body.as_deref(),
            Some("{\"id\":\"{{ uuid() }}\"}"),
            "file contents must be captured verbatim, template syntax intact"
        );
    }

    #[test]
    fn missing_body_file_is_rejected_at_construction() {
        let cfg = cfg_with(IndexMap::new(), Some("@/nonexistent/path/body.json"));
        let err = PreparedRequest::new(&cfg, &engine()).unwrap_err();
        assert!(
            err.to_string().contains("failed to load body file"),
            "a missing body file must fail loudly, not produce empty bodies: {err}"
        );
    }

    #[test]
    fn body_path_with_quote_does_not_break_template_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("we\"ird.json");
        std::fs::write(&path, "payload").unwrap();

        let arg = format!("@{}", path.display());
        let cfg = cfg_with(IndexMap::new(), Some(&arg));
        let p = prepared(&cfg);

        assert_eq!(p.static_body.as_deref(), Some("payload"));
    }

    #[test]
    fn templated_inline_body_is_rejected() {
        let cfg = cfg_with(IndexMap::new(), Some("{\"id\":\"{{ uuid() }}\"}"));
        let err = PreparedRequest::new(&cfg, &engine()).unwrap_err();
        assert!(err.to_string().contains("templated inline bodies"));
    }

    #[test]
    fn invalid_header_name_is_skipped_not_fatal() {
        let mut headers = IndexMap::new();
        headers.insert("bad header name".to_string(), "v".to_string());
        assert!(prepared(&cfg_with(headers, None)).static_headers.is_empty());
    }

    #[test]
    fn invalid_method_falls_back_to_get() {
        let mut cfg = cfg_with(IndexMap::new(), None);
        cfg.method = "not a method".to_string();
        assert_eq!(prepared(&cfg).method, reqwest::Method::GET);
    }

    #[test]
    fn build_rejects_empty_url() {
        let mut cfg = cfg_with(IndexMap::new(), None);
        cfg.url = String::new();
        let p = prepared(&cfg);
        let client = reqwest::Client::new();
        let err = p
            .build(&client, &engine(), &TemplateContext::default())
            .unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn build_renders_url_template() {
        let mut cfg = cfg_with(IndexMap::new(), None);
        cfg.url = "http://127.0.0.1:9/u/{{ user_id }}".into();
        let p = prepared(&cfg);
        let client = reqwest::Client::new();
        let ctx = TemplateContext::new("alice".into());

        let request = p.build(&client, &engine(), &ctx).unwrap().build().unwrap();
        assert_eq!(request.url().as_str(), "http://127.0.0.1:9/u/alice");
    }
}
