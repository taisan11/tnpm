//! Production defaults plus opt-in, runtime-only transport controls for diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transport {
    #[default]
    Default,
    Baseline,
    Gzip,
    Http2,
    Both,
}

impl Transport {
    pub fn current() -> Self {
        if std::env::var("TNPM_PROFILE").as_deref() == Ok("1") {
            return std::env::var("TNPM_PROFILE_TRANSPORT")
                .ok()
                .as_deref()
                .and_then(Self::named)
                .unwrap_or_default();
        }
        Self::Default
    }

    fn named(name: &str) -> Option<Self> {
        match name {
            "default" => Some(Self::Default),
            "baseline" => Some(Self::Baseline),
            "gzip" => Some(Self::Gzip),
            "http2" => Some(Self::Http2),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Baseline => "baseline",
            Self::Gzip => "gzip",
            Self::Http2 => "http2",
            Self::Both => "both",
        }
    }

    pub fn accept_encoding(self) -> &'static str {
        match self {
            Self::Default => "gzip, br, zstd, deflate",
            Self::Gzip | Self::Both => "gzip",
            Self::Baseline | Self::Http2 => "identity",
        }
    }

    pub fn configure(self, mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        if self != Self::Default {
            builder = builder
                .gzip(matches!(self, Self::Gzip | Self::Both))
                .no_brotli()
                .no_zstd()
                .no_deflate();
            if matches!(self, Self::Baseline | Self::Gzip) {
                builder = builder.http1_only();
            }
        }
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_advertises_all_enabled_decoders() {
        assert_eq!(
            Transport::default().accept_encoding(),
            "gzip, br, zstd, deflate"
        );
    }

    #[test]
    fn diagnostic_modes_have_explicit_encoding_policies() {
        for (name, encoding) in [
            ("baseline", "identity"),
            ("gzip", "gzip"),
            ("http2", "identity"),
            ("both", "gzip"),
        ] {
            let transport = Transport::named(name).unwrap();
            assert_eq!(transport.label(), name);
            assert_eq!(transport.accept_encoding(), encoding);
        }
        assert!(Transport::named("unknown").is_none());
    }
}
