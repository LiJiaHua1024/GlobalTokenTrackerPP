//! Cross-source price consensus.
//!
//! Every feed we read is third-party and every one of them is wrong
//! sometimes: measured against each other on the same model, LiteLLM agrees
//! with the other sources only ~50–65% of the time (it mixes batch / flex /
//! regional-endpoint prices), and single rows are off by 2×–20× often enough
//! to matter (`gpt-5` at half price, `gpt-oss-120b` at $2.92, a zero-price row
//! for a model that costs $12). Picking a winner by fixed source precedence
//! turns any one feed's mistake into a wrong bill.
//!
//! So a model's price is decided by **vote**:
//!
//! 1. Quotes are grouped by *canonical* id — `claude-opus-4.6`,
//!    `claude-opus-4-6` and `anthropic/claude-opus-4.6` are one model, whatever
//!    a feed's spelling of the version separator.
//! 2. Rows of zero cost are "no data", not a vote for free (models.dev and the
//!    bundled seed list 0/0 for models that plainly cost money). The bundled
//!    seed only votes when no live source knows the model — it is a snapshot
//!    of one of the live feeds, not an independent witness.
//! 3. Voters are clustered by (input, output) within [`TOLERANCE`]. The
//!    heaviest cluster with at least two members wins; its numbers come from
//!    its most trusted member. Two voters that disagree → the more trusted one.
//!    Three or more that all disagree → the median by output price, so an
//!    outlier at either end can never win.
//! 4. The result records how many sources agree out of how many voted, and
//!    what each one said, so the UI can show corroboration and flag disputes
//!    instead of hiding them.

use super::{Price, normalize_key};
use anyhow::Result;
use rusqlite::Connection;
use std::collections::HashMap;

/// Two quotes agree when input **and** output are within this fraction of
/// each other. Wide enough for rounding and the odd 5% reseller fee, narrow
/// enough that regional (+10–20%), batch (−50%) and flex prices are dissent.
pub const TOLERANCE: f64 = 0.06;

/// Sources by trust: earlier wins ties and supplies the numbers of the winning
/// cluster; the weight is the vote's strength. Weights discount feeds that
/// are partly derived from others (llmpricing.dev republishes models.dev and
/// OpenRouter) or known to mix price modes (LiteLLM). Unlisted sources rank
/// last with weight 0.5.
const TRUST: &[(&str, f64)] = &[
    ("portkey", 1.0),
    ("langfuse", 1.0),
    ("llm-prices", 1.0),
    ("openrouter", 1.0),
    ("vercel", 1.0),
    ("models.dev", 1.0),
    ("helicone", 0.9),
    ("litellm", 0.8),
    ("llmpricing", 0.6),
    ("seed", 0.0),
];

fn rank(source: &str) -> usize {
    TRUST
        .iter()
        .position(|(s, _)| *s == source)
        .unwrap_or(TRUST.len())
}

fn weight(source: &str) -> f64 {
    TRUST
        .iter()
        .find(|(s, _)| *s == source)
        .map_or(0.5, |(_, w)| *w)
}

/// Spelling-insensitive id: [`normalize_key`], then a dot between digits
/// becomes a hyphen (`claude-opus-4.6` ≡ `claude-opus-4-6`,
/// `qwen2.5-72b` ≡ `qwen2-5-72b`).
pub fn canon_key(raw: &str) -> String {
    let k = normalize_key(raw);
    let b = k.as_bytes();
    let mut out = String::with_capacity(k.len());
    for (i, ch) in k.char_indices() {
        let between_digits = ch == '.'
            && i > 0
            && b[i - 1].is_ascii_digit()
            && b.get(i + 1).is_some_and(u8::is_ascii_digit);
        out.push(if between_digits { '-' } else { ch });
    }
    out
}

/// One source's quote for one spelling of a model id.
#[derive(Debug, Clone)]
pub struct Quote {
    pub source: String,
    /// The id as stored (normalized, original version separators).
    pub key: String,
    pub price: Price,
}

/// How a quote relates to the price that was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Stance {
    /// Within tolerance of the chosen price.
    Agrees,
    /// Voted, and said something else.
    Dissents,
    /// Zero-price row: absence of data, not a vote.
    NoData,
    /// Bundled seed while live sources exist: not an independent witness.
    Ignored,
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub price: Price,
    /// The most trusted source among those behind [`Verdict::price`].
    pub source: String,
    pub agree: usize,
    /// Sources that voted (a group nobody priced counts its rows instead).
    pub total: usize,
}

/// A model as the sources collectively see it.
#[derive(Debug, Clone)]
pub struct Group {
    pub canon: String,
    /// Every spelling seen, display spelling first.
    pub keys: Vec<String>,
    /// Most trusted source first.
    pub quotes: Vec<Quote>,
    pub stances: Vec<Stance>,
    pub verdict: Verdict,
}

fn usable(p: &Price) -> bool {
    p.input.is_finite()
        && p.output.is_finite()
        && p.input >= 0.0
        && p.output >= 0.0
        && (p.input > 0.0 || p.output > 0.0)
}

fn near(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= TOLERANCE * a.max(b)
}

fn close(a: &Price, b: &Price) -> bool {
    near(a.input, b.input) && near(a.output, b.output)
}

/// A source that lists a model under two spellings (`claude-opus-4-6` and
/// `claude-opus-4.6`, sometimes at different prices — LiteLLM keeps regional
/// and plain routes side by side) still gets **one** vote: the spelling that
/// the most *other* sources corroborate, the first (by id) on a tie.
/// `candidates` are indices into `quotes`, ascending; the result keeps that
/// order.
fn one_vote_per_source(quotes: &[Quote], candidates: &[usize]) -> Vec<usize> {
    candidates
        .iter()
        .copied()
        .filter(|&i| {
            let src = &quotes[i].source;
            let rivals: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&j| quotes[j].source == *src)
                .collect();
            if rivals.len() == 1 {
                return true;
            }
            let support = |k: usize| {
                let mut seen: Vec<&str> = Vec::new();
                for &o in candidates {
                    let other = quotes[o].source.as_str();
                    if other != src
                        && !seen.contains(&other)
                        && close(&quotes[k].price, &quotes[o].price)
                    {
                        seen.push(other);
                    }
                }
                seen.len()
            };
            // `max_by_key` keeps the last maximum; rivals are in id order and
            // the *first* should win a tie, hence the reversed scan.
            let best = rivals
                .iter()
                .rev()
                .copied()
                .max_by_key(|&k| support(k))
                .unwrap_or(i);
            best == i
        })
        .collect()
}

/// Decide the price for one model. `quotes` must be ordered most trusted
/// first (see [`Group::quotes`]).
pub fn decide(quotes: &[Quote]) -> (Verdict, Vec<Stance>) {
    let live = quotes.iter().any(|q| q.source != "seed");
    let votes = |q: &Quote| (q.source != "seed" || !live) && usable(&q.price);
    let candidates: Vec<usize> = (0..quotes.len()).filter(|&i| votes(&quotes[i])).collect();
    let idx = one_vote_per_source(quotes, &candidates);

    let stance_all = |f: &dyn Fn(usize) -> Stance| (0..quotes.len()).map(f).collect::<Vec<_>>();

    if idx.is_empty() {
        // Nobody priced it: free, or simply unknown — the zero price stands,
        // with every row counting as "agreeing" that there is nothing here.
        let rep = quotes.first().map_or("seed", |q| q.source.as_str());
        let n = quotes
            .iter()
            .filter(|q| q.source != "seed" || !live)
            .count();
        return (
            Verdict {
                price: Price::default(),
                source: rep.to_string(),
                agree: n,
                total: n,
            },
            stance_all(&|i| {
                if quotes[i].source == "seed" && live {
                    Stance::Ignored
                } else {
                    Stance::NoData
                }
            }),
        );
    }

    // Greedy clusters, seeded by the most trusted quote not yet placed.
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for &i in &idx {
        match clusters
            .iter_mut()
            .find(|c| close(&quotes[c[0]].price, &quotes[i].price))
        {
            Some(c) => c.push(i),
            None => clusters.push(vec![i]),
        }
    }
    let power = |c: &Vec<usize>| c.iter().map(|&i| weight(&quotes[i].source)).sum::<f64>();

    let best = clusters
        .iter()
        .enumerate()
        .max_by(|(ia, a), (ib, b)| {
            power(a)
                .partial_cmp(&power(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                // earlier cluster = more trusted seed wins a tie
                .then(ib.cmp(ia))
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    let winner: &Vec<usize> = if clusters[best].len() >= 2 || idx.len() == 1 {
        &clusters[best]
    } else if idx.len() == 2 {
        &clusters[0]
    } else {
        // Everyone disagrees: the median by output, then input.
        let mut order: Vec<usize> = (0..clusters.len()).collect();
        order.sort_by(|&a, &b| {
            let (pa, pb) = (&quotes[clusters[a][0]].price, &quotes[clusters[b][0]].price);
            pa.output
                .partial_cmp(&pb.output)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    pa.input
                        .partial_cmp(&pb.input)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.cmp(&b))
        });
        &clusters[order[order.len() / 2]]
    };

    let rep = &quotes[winner[0]];
    let mut price = rep.price;
    // Fill what the representative lacks from its agreeing peers.
    let first = |f: fn(&Price) -> Option<f64>| {
        winner
            .iter()
            .find_map(|&i| f(&quotes[i].price).filter(|v| *v > 0.0))
    };
    if price.cache_read <= 0.0 {
        price.cache_read = first(|p| Some(p.cache_read)).unwrap_or(0.0);
    }
    if price.cache_write <= 0.0 {
        price.cache_write = first(|p| Some(p.cache_write)).unwrap_or(0.0);
    }
    price.tier_above_200k_input = price
        .tier_above_200k_input
        .or_else(|| first(|p| p.tier_above_200k_input));
    price.tier_1h_cache_write = price
        .tier_1h_cache_write
        .or_else(|| first(|p| p.tier_1h_cache_write));
    price.tier_batch = price.tier_batch.or_else(|| first(|p| p.tier_batch));

    let stances = stance_all(&|i| {
        let q = &quotes[i];
        if q.source == "seed" && live {
            Stance::Ignored
        } else if votes(q) && !idx.contains(&i) {
            // The same source's other spelling of this model: it already voted.
            Stance::Ignored
        } else if !usable(&q.price) {
            Stance::NoData
        } else if close(&q.price, &price) {
            Stance::Agrees
        } else {
            Stance::Dissents
        }
    });
    let agree = stances.iter().filter(|s| **s == Stance::Agrees).count();
    let total = agree + stances.iter().filter(|s| **s == Stance::Dissents).count();
    (
        Verdict {
            price,
            source: rep.source.clone(),
            agree,
            total,
        },
        stances,
    )
}

/// Read every stored quote, group by canonical id and decide each group.
/// Sorted by display id.
pub fn groups(conn: &Connection) -> Result<Vec<Group>> {
    let mut st = conn.prepare(
        "SELECT model_id, input, output, cache_read, cache_write,
                tier_above_200k_input, tier_1h_cache_write, tier_batch, source
         FROM prices",
    )?;
    let mut by_canon: HashMap<String, Vec<Quote>> = HashMap::new();
    let rows = st.query_map([], |r| {
        let f = |i: usize| r.get::<_, Option<f64>>(i);
        Ok(Quote {
            key: r.get(0)?,
            price: Price {
                input: f(1)?.unwrap_or(0.0),
                output: f(2)?.unwrap_or(0.0),
                cache_read: f(3)?.unwrap_or(0.0),
                cache_write: f(4)?.unwrap_or(0.0),
                tier_above_200k_input: f(5)?,
                tier_1h_cache_write: f(6)?,
                tier_batch: f(7)?,
            },
            source: r.get(8)?,
        })
    })?;
    for q in rows {
        let q = q?;
        let canon = canon_key(&q.key);
        if canon.is_empty() {
            continue; // a feed's blank id names no model
        }
        by_canon.entry(canon).or_default().push(q);
    }
    let mut out: Vec<Group> = by_canon
        .into_iter()
        .map(|(canon, mut quotes)| {
            // Trust order; ties (a source under two spellings) by id, so the
            // outcome never depends on row order.
            quotes.sort_by(|a, b| {
                rank(&a.source)
                    .cmp(&rank(&b.source))
                    .then_with(|| a.key.cmp(&b.key))
            });
            let (verdict, stances) = decide(&quotes);
            let keys = display_order(&quotes);
            Group {
                canon,
                keys,
                quotes,
                stances,
                verdict,
            }
        })
        .collect();
    out.sort_by(|a, b| a.keys[0].cmp(&b.keys[0]));
    Ok(out)
}

/// Distinct spellings, the one most sources use first (ties: the
/// lexicographically smaller — hyphens sort before dots, so `4-6` beats `4.6`).
fn display_order(quotes: &[Quote]) -> Vec<String> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for q in quotes {
        *count.entry(q.key.as_str()).or_default() += 1;
    }
    let mut keys: Vec<&str> = count.keys().copied().collect();
    keys.sort_by(|a, b| count[b].cmp(&count[a]).then(a.cmp(b)));
    keys.into_iter().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(source: &str, key: &str, input: f64, output: f64) -> Quote {
        Quote {
            source: source.into(),
            key: key.into(),
            price: Price {
                input,
                output,
                ..Default::default()
            },
        }
    }

    /// `decide` wants trust order, exactly what `groups` provides.
    fn ordered(mut v: Vec<Quote>) -> Vec<Quote> {
        v.sort_by_key(|q| rank(&q.source));
        v
    }

    fn verdict(v: Vec<Quote>) -> (Verdict, Vec<Stance>) {
        decide(&ordered(v))
    }

    #[test]
    fn canonical_ids_ignore_version_separators() {
        for (a, b) in [
            ("claude-opus-4.6", "claude-opus-4-6"),
            ("anthropic/claude-opus-4.6", "claude-opus-4-6"),
            ("qwen2.5-72b", "qwen2-5-72b"),
            ("Gemini-3.1-Pro", "gemini-3-1-pro"),
            ("mistral-7b-v0.1", "mistral-7b-v0-1"),
        ] {
            assert_eq!(canon_key(a), canon_key(b), "{a} vs {b}");
        }
        // Only a dot *between digits* is a version separator.
        assert_ne!(canon_key("gpt-4.turbo"), canon_key("gpt-4-turbo"));
        assert_eq!(canon_key("llama-3.1-8b"), "llama-3-1-8b");
        // Genuinely different models stay different.
        assert_ne!(canon_key("gpt-5"), canon_key("gpt-5-mini"));
        assert_ne!(
            canon_key("gemini-3.1-pro"),
            canon_key("gemini-3.1-pro-preview")
        );
    }

    #[test]
    fn a_majority_outvotes_a_single_wrong_feed() {
        // The real `gpt-5` case: LiteLLM at half price.
        let (v, st) = verdict(vec![
            q("portkey", "gpt-5", 1.25, 10.0),
            q("langfuse", "gpt-5", 1.25, 10.0),
            q("models.dev", "gpt-5", 1.25, 10.0),
            q("litellm", "gpt-5", 0.625, 5.0),
        ]);
        assert_eq!((v.price.input, v.price.output), (1.25, 10.0));
        assert_eq!((v.agree, v.total), (3, 4));
        assert_eq!(v.source, "portkey");
        // Ordered by trust: portkey, langfuse, models.dev, litellm.
        assert_eq!(
            st,
            [
                Stance::Agrees,
                Stance::Agrees,
                Stance::Agrees,
                Stance::Dissents
            ]
        );
    }

    #[test]
    fn a_lone_high_precedence_outlier_cannot_win() {
        // llmpricing used to outrank everything: `gpt-oss-120b` at $2.92.
        let (v, _) = verdict(vec![
            q("llmpricing", "m", 2.92, 2.92),
            q("litellm", "m", 0.15, 0.6),
            q("seed", "m", 0.15, 0.6),
            q("openrouter", "m", 0.15, 0.6),
        ]);
        assert_eq!((v.price.input, v.price.output), (0.15, 0.6));
        assert_eq!((v.agree, v.total), (2, 3)); // seed is not a witness
    }

    #[test]
    fn near_equal_prices_agree_and_the_trusted_number_is_used() {
        let (v, st) = verdict(vec![
            q("openrouter", "m", 0.28, 0.42),
            q("litellm", "m", 0.27, 0.41),
            q("vercel", "m", 0.62, 1.85),
        ]);
        // The two close quotes form the winning cluster, in the numbers of
        // the more trusted one; the third is dissent.
        assert_eq!((v.price.input, v.price.output), (0.28, 0.42));
        assert_eq!((v.agree, v.total), (2, 3));
        assert_eq!(v.source, "openrouter");
        assert_eq!(st, [Stance::Agrees, Stance::Dissents, Stance::Agrees]);
    }

    #[test]
    fn the_tolerance_is_what_separates_rounding_from_dissent() {
        let within = verdict(vec![
            q("portkey", "m", 5.0, 25.0),
            q("openrouter", "m", 5.2, 26.0),
        ]);
        assert_eq!((within.0.agree, within.0.total), (2, 2));
        let beyond = verdict(vec![
            q("portkey", "m", 5.0, 25.0),
            q("litellm", "m", 6.0, 30.0),
        ]);
        // Bedrock's +20% regional price is dissent; the more trusted source wins.
        assert_eq!((beyond.0.price.input, beyond.0.price.output), (5.0, 25.0));
        assert_eq!((beyond.0.agree, beyond.0.total), (1, 2));
    }

    #[test]
    fn three_way_disagreement_takes_the_median_never_an_extreme() {
        let (v, st) = verdict(vec![
            q("portkey", "m", 1.0, 3.0),
            q("openrouter", "m", 0.5, 1.0),
            q("litellm", "m", 4.0, 9.0),
        ]);
        assert_eq!((v.price.input, v.price.output), (1.0, 3.0));
        assert_eq!((v.agree, v.total), (1, 3));
        assert_eq!(st.iter().filter(|s| **s == Stance::Dissents).count(), 2);
    }

    #[test]
    fn zero_rows_are_absence_not_a_vote_for_free() {
        // models.dev and the seed list 0/0 for a model that costs money.
        let (v, st) = verdict(vec![
            q("models.dev", "gemini-3-5-flash", 0.0, 0.0),
            q("openrouter", "gemini-3.5-flash", 1.5, 9.0),
            q("vercel", "gemini-3.5-flash", 1.5, 9.0),
            q("seed", "gemini-3-5-flash", 0.0, 0.0),
        ]);
        assert_eq!((v.price.input, v.price.output), (1.5, 9.0));
        assert_eq!((v.agree, v.total), (2, 2));
        assert!(st.contains(&Stance::NoData));
        assert!(st.contains(&Stance::Ignored));
    }

    #[test]
    fn when_only_zero_rows_exist_the_model_is_free() {
        let (v, st) = verdict(vec![
            q("models.dev", "free-m", 0.0, 0.0),
            q("litellm", "free-m", 0.0, 0.0),
        ]);
        assert_eq!((v.price.input, v.price.output), (0.0, 0.0));
        assert_eq!((v.agree, v.total), (2, 2));
        assert!(st.iter().all(|s| *s == Stance::NoData));
    }

    #[test]
    fn the_seed_only_speaks_when_no_live_source_knows_the_model() {
        let (v, _) = verdict(vec![q("seed", "offline-m", 3.0, 15.0)]);
        assert_eq!((v.price.input, v.price.output), (3.0, 15.0));
        assert_eq!((v.agree, v.total), (1, 1));
        // …but never outvotes a live source, even when it repeats another one.
        let (v, st) = verdict(vec![
            q("models.dev", "m", 2.0, 8.0),
            q("seed", "m", 9.0, 9.0),
            q("seed", "m2", 9.0, 9.0),
        ]);
        assert_eq!((v.price.input, v.price.output), (2.0, 8.0));
        assert_eq!(st.iter().filter(|s| **s == Stance::Ignored).count(), 2);
    }

    #[test]
    fn cache_prices_and_tiers_come_from_the_agreeing_sources_only() {
        let mut lite = q("litellm", "m", 5.0, 25.0);
        lite.price.cache_read = 0.5;
        lite.price.cache_write = 6.25;
        lite.price.tier_above_200k_input = Some(10.0);
        let mut rogue = q("llmpricing", "m", 50.0, 250.0);
        rogue.price.cache_read = 5.0;
        rogue.price.tier_batch = Some(25.0);
        let (v, _) = verdict(vec![
            q("portkey", "m", 5.0, 25.0),
            q("langfuse", "m", 5.0, 25.0),
            lite,
            rogue,
        ]);
        // The representative (portkey) has none: they are filled in from its
        // agreeing peers — and not from the dissenting feed.
        assert_eq!(v.price.cache_read, 0.5);
        assert_eq!(v.price.cache_write, 6.25);
        assert_eq!(v.price.tier_above_200k_input, Some(10.0));
        assert_eq!(v.price.tier_batch, None);
    }

    #[test]
    fn input_only_models_are_still_priced() {
        // Embedding models bill input only.
        let (v, _) = verdict(vec![
            q("openrouter", "emb", 0.02, 0.0),
            q("vercel", "emb", 0.02, 0.0),
        ]);
        assert_eq!((v.price.input, v.price.output), (0.02, 0.0));
        assert_eq!((v.agree, v.total), (2, 2));
    }

    #[test]
    fn garbage_numbers_never_vote() {
        let (v, st) = verdict(vec![
            q("portkey", "m", f64::NAN, 1.0),
            q("openrouter", "m", -1.0, 1.0),
            q("vercel", "m", 2.0, 4.0),
        ]);
        assert_eq!((v.price.input, v.price.output), (2.0, 4.0));
        assert_eq!((v.agree, v.total), (1, 1));
        assert_eq!(st.iter().filter(|s| **s == Stance::NoData).count(), 2);
    }

    #[test]
    fn a_source_counts_once_even_when_it_lists_two_spellings() {
        // LiteLLM keeps a plain and a regional route for the same model.
        let (v, st) = verdict(vec![
            q("portkey", "claude-opus-4-6", 5.0, 25.0),
            q("openrouter", "claude-opus-4.6", 5.0, 25.0),
            q("litellm", "claude-opus-4-6", 5.0, 25.0),
            q("litellm", "claude-opus-4.6", 2.5, 12.5),
        ]);
        assert_eq!((v.price.input, v.price.output), (5.0, 25.0));
        // Three sources voted, not four — and the odd spelling isn't dissent.
        assert_eq!((v.agree, v.total), (3, 3));
        assert_eq!(st.iter().filter(|s| **s == Stance::Ignored).count(), 1);

        // The better-corroborated spelling is the one that votes, whichever
        // id sorts first.
        let (v, _) = verdict(vec![
            q("models.dev", "deepseek-v3-2", 0.57, 1.71),
            q("models.dev", "deepseek-v3.2", 0.27, 0.42),
            q("openrouter", "deepseek-v3.2", 0.28, 0.42),
        ]);
        assert_eq!((v.price.input, v.price.output), (0.28, 0.42));
        assert_eq!((v.agree, v.total), (2, 2));
    }

    #[test]
    fn groups_merge_spellings_and_prefer_the_common_one() {
        let s = crate::store::Store::open_memory().unwrap();
        let c = s.conn();
        c.execute("DELETE FROM prices", []).unwrap();
        let put = |src: &str, id: &str, i: f64, o: f64| {
            c.execute(
                "INSERT INTO prices(provider, model_id, input, output, source, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?1, 0)",
                rusqlite::params![src, id, i, o],
            )
            .unwrap();
        };
        put("openrouter", "claude-opus-4.6", 5.0, 25.0);
        put("vercel", "claude-opus-4.6", 5.0, 25.0);
        put("litellm", "claude-opus-4-6", 5.0, 25.0);
        put("seed", "claude-opus-4-6", 0.0, 0.0);
        put("portkey", "gpt-5", 1.25, 10.0);
        // A spelling most sources share is the display one.
        put("openrouter", "gemini-3.1-pro", 2.0, 12.0);
        put("vercel", "gemini-3.1-pro", 2.0, 12.0);
        put("seed", "gemini-3-1-pro", 0.0, 0.0);
        let g = groups(c).unwrap();
        assert_eq!(g.len(), 3);
        let gem = g.iter().find(|g| g.canon == "gemini-3-1-pro").unwrap();
        assert_eq!(gem.keys[0], "gemini-3.1-pro");
        let opus = g.iter().find(|g| g.canon == "claude-opus-4-6").unwrap();
        // 2 quotes each: the tie goes to the hyphen spelling.
        assert_eq!(opus.keys, ["claude-opus-4-6", "claude-opus-4.6"]);
        assert_eq!((opus.verdict.agree, opus.verdict.total), (3, 3));
        // Trust order: openrouter, vercel, litellm, then the seed.
        let srcs: Vec<_> = opus.quotes.iter().map(|q| q.source.as_str()).collect();
        assert_eq!(srcs, ["openrouter", "vercel", "litellm", "seed"]);
        assert_eq!(opus.stances[3], Stance::Ignored);
    }

    /// What the consensus makes of a real, refreshed ledger — for eyeballing
    /// after adding a source: `GTT_DB=<ledger.db> cargo test -p
    /// globaltokentracker-core real_book_report -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs GTT_DB pointing at a refreshed ledger"]
    fn real_book_report() {
        let Some(db) = std::env::var_os("GTT_DB") else {
            eprintln!("GTT_DB not set — skipped");
            return;
        };
        let conn =
            Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let t = std::time::Instant::now();
        let g = groups(&conn).unwrap();
        eprintln!("groups(): {:?} for {} models", t.elapsed(), g.len());
        let (mut unanimous, mut majority, mut disputed, mut single, mut none) = (0, 0, 0, 0, 0);
        for x in &g {
            match (x.verdict.agree, x.verdict.total) {
                (_, 0) => none += 1,
                (a, t) if t == 1 && a == 1 => single += 1,
                (a, t) if a == t => unanimous += 1,
                (a, t) if a * 2 > t => majority += 1,
                _ => disputed += 1,
            }
        }
        eprintln!(
            "{} models: {unanimous} unanimous, {majority} majority, {disputed} disputed, {single} single-source, {none} unpriced",
            g.len()
        );
        for name in [
            "gpt-5",
            "gpt-oss-120b",
            "claude-opus-4-6",
            "gemini-3-1-pro",
            "gemini-3-5-flash",
            "deepseek-v3-2",
            "kimi-k2-5",
            "glm-5",
        ] {
            let Some(x) = g.iter().find(|x| x.canon == name) else {
                eprintln!("{name}: (absent)");
                continue;
            };
            eprintln!(
                "{name}: {:.4}/{:.4} via {} ({}/{})",
                x.verdict.price.input,
                x.verdict.price.output,
                x.verdict.source,
                x.verdict.agree,
                x.verdict.total
            );
            for (q, st) in x.quotes.iter().zip(&x.stances) {
                eprintln!(
                    "    {:11} {:>9.4} {:>9.4}  {:?}",
                    q.source, q.price.input, q.price.output, st
                );
            }
        }
        assert!(g.len() > 1000);
    }
}
