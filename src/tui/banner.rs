/// ASCII art banner for Rustress — shown only on `--help`.
///
/// Block letters with a gradient-like dual-tone (cyan → purple).
pub fn banner() -> String {
    let version = crate::core::constants::VERSION;
    format!(
        "\x1b[38;2;196;248;245m██████╗ ██╗   ██╗███████╗████████╗██████╗ ███████╗███████╗\x1b[0m
\x1b[38;2;173;226;224m██╔══██╗██║   ██║██╔════╝╚══██╔══╝██╔══██╗██╔════╝██╔════╝\x1b[0m
\x1b[38;2;150;204;203m██████╔╝██║   ██║███████╗   ██║   ██████╔╝█████╗  ███████╗\x1b[0m
\x1b[38;2;127;182;182m██╔══██╗██║   ██║╚════██║   ██║   ██╔══██╗██╔══╝  ╚════██║\x1b[0m
\x1b[38;2;104;160;161m██║  ██║╚██████╔╝███████║   ██║   ██║  ██║███████╗███████║\x1b[0m
\x1b[38;2;79;137;139m╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝   ╚═╝  ╚═╝╚══════╝╚══════╝\x1b[0m
\x1b[38;2;196;248;245mv{version} | High-Performance Load Testing Engine\x1b[0m\n"
    )
}

/// Compact inline banner for CLI help (shorter width).
pub fn short_banner() -> String {
    let version = crate::core::constants::VERSION;
    format!(
        "\x1b[1;36m  RUSTRESS\x1b[0m \x1b[38;5;147mv{version}\x1b[0m\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_banner_contains_rustress() {
        let b = banner();
        assert!(b.contains("RUSTRESS") || b.contains("████"));
    }

    #[test]
    fn test_banner_has_version() {
        let b = banner();
        assert!(b.contains("v0.1.0"));
    }

    #[test]
    fn test_short_banner_not_empty() {
        let b = short_banner();
        assert!(b.contains("RUSTRESS"));
    }
}
