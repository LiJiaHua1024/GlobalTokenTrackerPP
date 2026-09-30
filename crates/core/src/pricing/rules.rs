//! Pricing rules as data: feed list, source trust, matching peels, routing
//! placeholders, aliases and the long-context threshold. The bundled copy
//! (`assets/pricing_rules.json`) is the floor; a newer copy published in the
//! repo is fetched at refresh and wins when its `revision` is at least the
//! bundled one, so a rules change ships without a new installer.

use crate::store::Store;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SUPPORTED_SCHEMA: u32 = 1;
/// app_state key holding the last valid remote rules document (raw JSON).
pub const STATE_KEY: &str = "pricing_rules";
pub const RULES_URL: &str = "https://raw.githubusercontent.com/jichuo1/GlobalTokenTracker/main/crates/core/assets/pricing_rules.json";
/// Overrides [`RULES_URL`]; an empty value disables the remote fetch.
pub const RULES_URL_ENV: &str = "GTT_PRICING_RULES_URL";

const BUNDLED: &str = include_str!("../../assets/pricing_rules.json");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedSpec {
    pub tag: String,
    /// Importer name; an unknown one is skipped at refresh (forward compat).
    pub format: String,
    pub url: String,
    /// Portkey only: model-id prefixes this file may contribute (empty = all).
    #[serde(default)]
    pub prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustEntry {
    pub source: String,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    pub schema: u32,
    pub revision: u32,
    pub feeds: Vec<FeedSpec>,
    /// Most trusted first: position is the rank, `weight` the vote strength.
    pub trust: Vec<TrustEntry>,
    /// Weight of a source missing from `trust` (it also ranks last).
    pub unlisted_weight: f64,
    pub tolerance: f64,
    pub strip_prefixes: Vec<String>,
    pub strip_suffixes: Vec<String>,
    /// `rfind` anchors for tails like `us.anthropic.claude-…`.
    pub anchor_markers: Vec<String>,
    /// Lengths of a trailing all-digit `-<date>` segment that may be peeled.
    pub date_suffix_lengths: Vec<usize>,
    /// Router placeholders (`auto`) that name no real model.
    pub routing_models: Vec<String>,
    /// normalized raw model → normalized book key.
    pub aliases: BTreeMap<String, String>,
    pub long_context_threshold: u64,
}

impl Rules {
    pub fn bundled() -> Self {
        let r: Self = serde_json::from_str(BUNDLED).expect("bundled pricing_rules.json parses");
        r.validate().expect("bundled pricing_rules.json is valid");
        r
    }

    pub fn parse(json: &str) -> Result<Self> {
        let r: Self = serde_json::from_str(json).context("parse pricing rules")?;
        r.validate()?;
        Ok(r)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == SUPPORTED_SCHEMA,
            "unsupported rules schema {}",
            self.schema
        );
        ensure!(!self.feeds.is_empty(), "no feeds");
        for f in &self.feeds {
            ensure!(
                f.url.starts_with("https://"),
                "feed {} url is not https",
                f.tag
            );
        }
        ensure!(
            self.tolerance.is_finite() && self.tolerance > 0.0 && self.tolerance <= 0.5,
            "tolerance out of range"
        );
        ensure!(
            self.unlisted_weight.is_finite() && self.unlisted_weight >= 0.0,
            "bad unlisted_weight"
        );
        for t in &self.trust {
            ensure!(
                t.weight.is_finite() && t.weight >= 0.0,
                "bad weight for {}",
                t.source
            );
        }
        ensure!(
            self.long_context_threshold > 0,
            "bad long_context_threshold"
        );
        if self
            .date_suffix_lengths
            .iter()
            .any(|n| !(1..=8).contains(n))
        {
            bail!("date_suffix_lengths outside 1..=8");
        }
        Ok(())
    }

    /// Position in the trust order; unlisted sources rank last.
    pub fn rank(&self, source: &str) -> usize {
        self.trust
            .iter()
            .position(|t| t.source == source)
            .unwrap_or(self.trust.len())
    }

    pub fn weight(&self, source: &str) -> f64 {
        self.trust
            .iter()
            .find(|t| t.source == source)
            .map_or(self.unlisted_weight, |t| t.weight)
    }

    /// Identity of the effective rules — what a repricing marker records.
    pub fn fingerprint(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        Sha256::digest(json.as_bytes())
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                use std::fmt::Write as _;
                let _ = write!(s, "{b:02x}");
                s
            })
    }
}

/// Effective rules: the stored remote copy when valid and at least as new as
/// the bundled one, else the bundled copy.
pub fn current(store: &Store) -> Rules {
    let bundled = Rules::bundled();
    let remote = store
        .get_state(STATE_KEY)
        .ok()
        .flatten()
        .and_then(|raw| Rules::parse(&raw).ok());
    match remote {
        Some(r) if r.revision >= bundled.revision => r,
        _ => bundled,
    }
}

/// Where the remote rules come from; `None` = disabled.
pub fn remote_url() -> Option<String> {
    match std::env::var(RULES_URL_ENV) {
        Ok(u) if u.is_empty() => None,
        Ok(u) => Some(u),
        Err(_) => Some(RULES_URL.to_string()),
    }
}

/// Validate a downloaded document and keep it as the stored remote copy.
pub fn store_remote(store: &Store, raw: &str) -> Result<Rules> {
    let r = Rules::parse(raw)?;
    store.set_state(STATE_KEY, raw)?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_with(f: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut v: serde_json::Value = serde_json::from_str(BUNDLED).unwrap();
        f(&mut v);
        v.to_string()
    }

    #[test]
    fn bundled_reproduces_the_previous_hardcoded_config() {
        let r = Rules::bundled();
        assert_eq!(r.feeds.len(), 20);
        assert_eq!(r.feeds.iter().filter(|f| f.tag == "portkey").count(), 12);
        let dash = r
            .feeds
            .iter()
            .find(|f| f.url.ends_with("/dashscope.json"))
            .unwrap();
        assert_eq!(dash.prefixes, ["qwen", "qwq"]);
        let order: Vec<&str> = r.trust.iter().map(|t| t.source.as_str()).collect();
        assert_eq!(
            order,
            [
                "portkey",
                "langfuse",
                "llm-prices",
                "openrouter",
                "vercel",
                "models.dev",
                "helicone",
                "litellm",
                "llmpricing",
                "seed"
            ]
        );
        assert_eq!(r.weight("helicone"), 0.9);
        assert_eq!(r.weight("litellm"), 0.8);
        assert_eq!(r.weight("llmpricing"), 0.6);
        assert_eq!(r.weight("seed"), 0.0);
        assert_eq!(r.weight("nobody"), 0.5);
        assert_eq!(r.rank("portkey"), 0);
        assert_eq!(r.rank("nobody"), 10);
        assert_eq!(r.tolerance, 0.06);
        assert_eq!(r.long_context_threshold, 200_000);
    }

    #[test]
    fn remote_with_higher_revision_wins_lower_is_ignored() {
        let s = Store::open_memory().unwrap();
        assert_eq!(current(&s).revision, 1);
        let newer = json_with(|v| {
            v["revision"] = 5.into();
            v["tolerance"] = 0.1.into();
        });
        store_remote(&s, &newer).unwrap();
        let r = current(&s);
        assert_eq!((r.revision, r.tolerance), (5, 0.1));
        // A remote older than the bundled floor never applies.
        let older = json_with(|v| {
            v["revision"] = 0.into();
            v["tolerance"] = 0.2.into();
        });
        s.set_state(STATE_KEY, &older).unwrap();
        assert_eq!(current(&s).tolerance, 0.06);
        // Tie → remote.
        let tie = json_with(|v| v["tolerance"] = 0.07.into());
        s.set_state(STATE_KEY, &tie).unwrap();
        assert_eq!(current(&s).tolerance, 0.07);
    }

    #[test]
    fn invalid_remote_documents_are_ignored() {
        let s = Store::open_memory().unwrap();
        let bad = [
            json_with(|v| v["schema"] = 2.into()),
            json_with(|v| v["feeds"][0]["url"] = "http://models.dev/api.json".into()),
            json_with(|v| v["tolerance"] = 0.9.into()),
            json_with(|v| v["feeds"] = serde_json::json!([])),
            json_with(|v| v["long_context_threshold"] = 0.into()),
            json_with(|v| v["date_suffix_lengths"] = serde_json::json!([9])),
            json_with(|v| v["trust"][0]["weight"] = (-1.0).into()),
            "not json".to_string(),
        ];
        for raw in &bad {
            assert!(store_remote(&s, &raw.clone()).is_err(), "{raw:.80}");
            // Even if such a document is already stored, it never applies.
            s.set_state(STATE_KEY, raw).unwrap();
            assert_eq!(current(&s), Rules::bundled());
        }
    }

    #[test]
    fn unknown_feed_format_is_valid() {
        let raw = json_with(|v| {
            v["feeds"].as_array_mut().unwrap().push(serde_json::json!(
                {"tag": "future", "format": "hologram", "url": "https://example.com/x.json"}
            ));
        });
        assert!(Rules::parse(&raw).is_ok());
    }

    #[test]
    fn fingerprint_tracks_content() {
        let a = Rules::bundled();
        let mut b = a.clone();
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.aliases.insert("x".into(), "y".into());
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
