//! Price-feed importers: models.dev, LiteLLM, llmpricing.dev, and the six
//! independent cross-check feeds (OpenRouter, Vercel AI Gateway, Helicone,
//! Langfuse, llm-prices.com, Portkey).
//!
//! These used to download all three documents and parse each into a full
//! `serde_json::Value` tree — three multi-MB DOMs alive at once, a 50–80MB
//! heap spike on **every launch** (the startup price refresh). Now each feed is
//! parsed straight into a few typed fields (everything else is skipped without
//! being materialised) and written before the next one is fetched, so the peak
//! is a single document's worth of typed data.
//!
//! The feeds are third-party and their shapes drift, so parsing is *lenient*
//! exactly where the old `Value` code was: a number field of the wrong type is
//! "absent", a container of the wrong type is "empty" — never a parse error.
//! The `reference_*` functions in the tests are the original `Value`
//! implementations, and the equivalence tests pin the two to the same rows.

use super::normalize_key;
use anyhow::Result;
use rusqlite::Connection;
use serde::Deserialize;
use serde::de::value::MapAccessDeserializer;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::fmt;
use std::marker::PhantomData;

// ------------------------------------------------------- lenient primitives

/// Drain whatever the deserializer is positioned on without keeping any of it.
fn skip_seq<'de, A: SeqAccess<'de>>(mut a: A) -> Result<(), A::Error> {
    while a.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

fn skip_map<'de, A: MapAccess<'de>>(mut a: A) -> Result<(), A::Error> {
    while a.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

/// A JSON number as `f64`; any other JSON value → `None` (`Value::as_f64`).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Num(Option<f64>);

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Num;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_f64<E>(self, v: f64) -> Result<Num, E> {
                Ok(Num(Some(v)))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Num, E> {
                Ok(Num(Some(v as f64)))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Num, E> {
                Ok(Num(Some(v as f64)))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Num, E> {
                Ok(Num(None))
            }
            fn visit_str<E>(self, _: &str) -> Result<Num, E> {
                Ok(Num(None))
            }
            fn visit_unit<E>(self) -> Result<Num, E> {
                Ok(Num(None))
            }
            fn visit_none<E>(self) -> Result<Num, E> {
                Ok(Num(None))
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Num, D2::Error> {
                d.deserialize_any(self)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Num, A::Error> {
                skip_seq(a).map(|()| Num(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Num, A::Error> {
                skip_map(a).map(|()| Num(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// A JSON string; any other value → `None` (`Value::as_str`).
#[derive(Clone, Default, Debug, PartialEq)]
struct Text(Option<String>);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Text;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_str<E>(self, v: &str) -> Result<Text, E> {
                Ok(Text(Some(v.to_string())))
            }
            fn visit_string<E>(self, v: String) -> Result<Text, E> {
                Ok(Text(Some(v)))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Text, E> {
                Ok(Text(None))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Text, E> {
                Ok(Text(None))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Text, E> {
                Ok(Text(None))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Text, E> {
                Ok(Text(None))
            }
            fn visit_unit<E>(self) -> Result<Text, E> {
                Ok(Text(None))
            }
            fn visit_none<E>(self) -> Result<Text, E> {
                Ok(Text(None))
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Text, D2::Error> {
                d.deserialize_any(self)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Text, A::Error> {
                skip_seq(a).map(|()| Text(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Text, A::Error> {
                skip_map(a).map(|()| Text(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// `T` when the JSON value is an object; anything else → `None` — the typed
/// twin of indexing a `Value` that turns out not to be an object.
#[derive(Debug)]
struct Loose<T>(Option<T>);

impl<T> Default for Loose<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Loose<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Loose<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Loose<T>, A::Error> {
                T::deserialize(MapAccessDeserializer::new(a)).map(|t| Loose(Some(t)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Loose<T>, A::Error> {
                skip_seq(a).map(|()| Loose(None))
            }
            fn visit_str<E>(self, _: &str) -> Result<Loose<T>, E> {
                Ok(Loose(None))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Loose<T>, E> {
                Ok(Loose(None))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Loose<T>, E> {
                Ok(Loose(None))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Loose<T>, E> {
                Ok(Loose(None))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Loose<T>, E> {
                Ok(Loose(None))
            }
            fn visit_unit<E>(self) -> Result<Loose<T>, E> {
                Ok(Loose(None))
            }
            fn visit_none<E>(self) -> Result<Loose<T>, E> {
                Ok(Loose(None))
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Loose<T>, D2::Error> {
                d.deserialize_any(self)
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// The entries of a JSON object, **in document order** (the old code relied on
/// `serde_json`'s `preserve_order`: with `INSERT OR REPLACE` a later duplicate
/// key wins); anything that is not an object → no entries.
#[derive(Debug)]
struct Entries<T>(Vec<(String, T)>);

impl<T> Default for Entries<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Entries<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Entries<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Entries<T>, A::Error> {
                let mut out = Vec::new();
                while let Some(kv) = a.next_entry::<String, T>()? {
                    out.push(kv);
                }
                Ok(Entries(out))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Entries<T>, A::Error> {
                skip_seq(a).map(|()| Entries(Vec::new()))
            }
            fn visit_str<E>(self, _: &str) -> Result<Entries<T>, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Entries<T>, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Entries<T>, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Entries<T>, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Entries<T>, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_unit<E>(self) -> Result<Entries<T>, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_none<E>(self) -> Result<Entries<T>, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Entries<T>, D2::Error> {
                d.deserialize_any(self)
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

/// The elements of a JSON array; anything that is not an array → none.
#[derive(Debug)]
struct Items<T>(Vec<T>);

impl<T> Default for Items<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Items<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Items<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Items<T>, A::Error> {
                let mut out = Vec::new();
                while let Some(x) = a.next_element::<T>()? {
                    out.push(x);
                }
                Ok(Items(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Items<T>, A::Error> {
                skip_map(a).map(|()| Items(Vec::new()))
            }
            fn visit_str<E>(self, _: &str) -> Result<Items<T>, E> {
                Ok(Items(Vec::new()))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Items<T>, E> {
                Ok(Items(Vec::new()))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Items<T>, E> {
                Ok(Items(Vec::new()))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Items<T>, E> {
                Ok(Items(Vec::new()))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Items<T>, E> {
                Ok(Items(Vec::new()))
            }
            fn visit_unit<E>(self) -> Result<Items<T>, E> {
                Ok(Items(Vec::new()))
            }
            fn visit_none<E>(self) -> Result<Items<T>, E> {
                Ok(Items(Vec::new()))
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Items<T>, D2::Error> {
                d.deserialize_any(self)
            }
        }
        d.deserialize_any(V(PhantomData))
    }
}

// ------------------------------------------------------------ models.dev

/// `{provider: {models: {id: {cost: {input,output,cache_read,cache_write}}}}}`
#[derive(Deserialize, Default)]
struct MdProvider {
    #[serde(default)]
    models: Entries<Loose<MdModel>>,
}

#[derive(Deserialize, Default)]
struct MdModel {
    #[serde(default)]
    cost: Loose<MdCost>,
}

#[derive(Deserialize, Default)]
struct MdCost {
    #[serde(default)]
    input: Num,
    #[serde(default)]
    output: Num,
    #[serde(default)]
    cache_read: Num,
    #[serde(default)]
    cache_write: Num,
}

pub(super) fn import_models_dev(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let providers: Entries<Loose<MdProvider>> = serde_json::from_str(body)?;
    let mut st = conn.prepare(
        "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write, source, fetched_at)
         VALUES ('models.dev', ?1, ?2, ?3, ?4, ?5, 'models.dev', ?6)",
    )?;
    let mut n = 0usize;
    for (_, prov) in providers.0 {
        let Some(prov) = prov.0 else { continue };
        for (id, model) in prov.models.0 {
            let Some(MdModel {
                cost: Loose(Some(c)),
            }) = model.0
            else {
                continue; // no cost object
            };
        st.execute(rusqlite::params![
            normalize_key(&id),
            // sane(): inf/NaN/negative feed values become "absent" (0.0 in
            // this importer's convention) instead of landing in the ledger.
            sane(c.input.0).unwrap_or(0.0),
            sane(c.output.0).unwrap_or(0.0),
            sane(c.cache_read.0).unwrap_or(0.0),
            sane(c.cache_write.0).unwrap_or(0.0),
            now
        ])?;
            n += 1;
        }
    }
    Ok(n)
}

// --------------------------------------------------------------- LiteLLM

/// `{model: {input_cost_per_token, …}}` — all $/token, ×1e6 to $/1M.
#[derive(Deserialize, Default)]
struct LlEntry {
    #[serde(default)]
    input_cost_per_token: Num,
    #[serde(default)]
    output_cost_per_token: Num,
    #[serde(default)]
    cache_read_input_token_cost: Num,
    #[serde(default)]
    cache_creation_input_token_cost: Num,
    #[serde(default)]
    input_cost_per_token_above_200k_tokens: Num,
    #[serde(default)]
    cache_creation_input_token_cost_above_1hr: Num,
    #[serde(default)]
    input_cost_per_token_batches: Num,
    #[serde(default)]
    output_cost_per_token_above_200k_tokens: Num,
    #[serde(default)]
    cache_read_input_token_cost_above_200k_tokens: Num,
}

pub(super) fn import_litellm(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let entries: Entries<Loose<LlEntry>> = serde_json::from_str(body)?;
    let mut st = conn.prepare(
        "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write,
                tier_above_200k_input, tier_1h_cache_write, tier_batch,
                tier_above_200k_output, tier_above_200k_cache_read, source, fetched_at)
         VALUES ('litellm', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'litellm', ?11)",
    )?;
    let per_1m = |n: Num| n.0.map(|x| x * 1e6);
    let mut n = 0usize;
    for (id, entry) in entries.0 {
        let Some(e) = entry.0 else { continue };
        if e.input_cost_per_token.0.is_none() {
            continue; // skip spec entries ("sample_spec", defaults)
        }
        st.execute(rusqlite::params![
            normalize_key(&id),
            sane(per_1m(e.input_cost_per_token)).unwrap_or(0.0),
            sane(per_1m(e.output_cost_per_token)).unwrap_or(0.0),
            sane(per_1m(e.cache_read_input_token_cost)).unwrap_or(0.0),
            sane(per_1m(e.cache_creation_input_token_cost)).unwrap_or(0.0),
            per_1m(e.input_cost_per_token_above_200k_tokens),
            per_1m(e.cache_creation_input_token_cost_above_1hr),
            per_1m(e.input_cost_per_token_batches),
            per_1m(e.output_cost_per_token_above_200k_tokens),
            per_1m(e.cache_read_input_token_cost_above_200k_tokens),
            now
        ])?;
        n += 1;
    }
    Ok(n)
}

// ---------------------------------------------------------- llmpricing.dev

/// `{models:[{id, reference:{provider,input,output,cacheRead,official},
/// cheapest:{...}}]}` — already $/1M, no conversion. Books the official
/// `reference` quote; `cheapest` only fills gaps so a bargain host never
/// understates the user's actual provider cost. The source exposes no
/// cache-write price → the column stays NULL (honest absence, not a guessed
/// multiplier).
#[derive(Deserialize, Default)]
struct LpRoot {
    #[serde(default)]
    models: Items<Loose<LpModel>>,
}

#[derive(Deserialize, Default)]
struct LpModel {
    #[serde(default)]
    id: Text,
    #[serde(default)]
    reference: Loose<LpQuote>,
    #[serde(default)]
    cheapest: Loose<LpQuote>,
}

#[derive(Deserialize, Default)]
struct LpQuote {
    #[serde(default)]
    input: Num,
    #[serde(default)]
    output: Num,
    #[serde(default, rename = "cacheRead")]
    cache_read: Num,
}

pub(super) fn import_llmpricing(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let root: Loose<LpRoot> = serde_json::from_str(body)?;
    let mut st = conn.prepare(
        "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write, source, fetched_at)
         VALUES ('llmpricing', ?1, ?2, ?3, ?4, NULL, 'llmpricing', ?5)",
    )?;
    let mut n = 0usize;
    for m in root.0.unwrap_or_default().models.0 {
        let Some(m) = m.0 else { continue };
        let Some(id) = m.id.0 else { continue };
        let reference = m.reference.0.unwrap_or_default();
        let q = if reference.input.0.is_some() {
            reference
        } else {
            m.cheapest.0.unwrap_or_default()
        };
        if q.input.0.is_none() && q.output.0.is_none() {
            continue; // neither quote usable
        }
        st.execute(rusqlite::params![
            normalize_key(&id),
            sane(q.input.0).unwrap_or(0.0),
            sane(q.output.0).unwrap_or(0.0),
            sane(q.cache_read.0).unwrap_or(0.0),
            now
        ])?;
        n += 1;
    }
    Ok(n)
}

// ------------------------------------------------- independent cross-check feeds
//
// models.dev, LiteLLM and llmpricing.dev are not enough to trust a price: they
// disagree with each other on a fifth of the models they share, and llmpricing
// republishes some of the others. These six are curated or served by
// unrelated parties and mostly quote *first-party list prices* (gateways pass
// them through unchanged), which is what makes them useful as votes — see
// `consensus`. Every importer filters out the rows that would poison a vote:
// reseller markups, free-tier variants, long-context surcharge rows that share
// an id with the base model.

/// A JSON number, or a string holding one (`"0.00000075"`); anything else →
/// `None`. OpenRouter and Vercel quote decimal *strings*.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Amount(Option<f64>);

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Amount;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_f64<E>(self, v: f64) -> Result<Amount, E> {
                Ok(Amount(Some(v)))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Amount, E> {
                Ok(Amount(Some(v as f64)))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Amount, E> {
                Ok(Amount(Some(v as f64)))
            }
            fn visit_str<E>(self, v: &str) -> Result<Amount, E> {
                Ok(Amount(v.trim().parse::<f64>().ok()))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Amount, E> {
                Ok(Amount(None))
            }
            fn visit_unit<E>(self) -> Result<Amount, E> {
                Ok(Amount(None))
            }
            fn visit_none<E>(self) -> Result<Amount, E> {
                Ok(Amount(None))
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Amount, D2::Error> {
                d.deserialize_any(self)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Amount, A::Error> {
                skip_seq(a).map(|()| Amount(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Amount, A::Error> {
                skip_map(a).map(|()| Amount(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// A finite, non-negative price; a sentinel (`-1` = "dynamic"), NaN or a
/// negative number is "absent".
fn sane(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite() && *x >= 0.0)
}

/// Writes one source's rows: a fixed source tag, keys normalized, a quote with
/// neither an input nor an output price skipped.
struct Sink<'c> {
    st: rusqlite::Statement<'c>,
    source: &'static str,
    now: i64,
    n: usize,
}

impl<'c> Sink<'c> {
    fn new(conn: &'c Connection, source: &'static str, now: i64) -> Result<Self> {
        let st = conn.prepare(
            "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write, source, fetched_at)
             VALUES (?7, ?1, ?2, ?3, ?4, ?5, ?7, ?6)",
        )?;
        Ok(Self {
            st,
            source,
            now,
            n: 0,
        })
    }

    /// $/1M values. Cache columns stay NULL when the feed has no figure
    /// (honest absence, not a guessed multiplier).
    fn put(
        &mut self,
        id: &str,
        input: Option<f64>,
        output: Option<f64>,
        cache_read: Option<f64>,
        cache_write: Option<f64>,
    ) -> Result<()> {
        let (input, output) = (sane(input), sane(output));
        let key = normalize_key(id);
        if key.is_empty() || (input.is_none() && output.is_none()) {
            return Ok(());
        }
        self.st.execute(rusqlite::params![
            key,
            input.unwrap_or(0.0),
            output.unwrap_or(0.0),
            sane(cache_read),
            sane(cache_write),
            self.now,
            self.source,
        ])?;
        self.n += 1;
        Ok(())
    }
}

// -------------------------------------------------------------- OpenRouter

/// `{data:[{id, pricing:{prompt, completion, input_cache_read,
/// input_cache_write}}]}` — decimal strings, $/token.
#[derive(Deserialize, Default)]
struct OrRoot {
    #[serde(default)]
    data: Items<Loose<OrModel>>,
}

#[derive(Deserialize, Default)]
struct OrModel {
    #[serde(default)]
    id: Text,
    #[serde(default)]
    pricing: Loose<OrPricing>,
}

#[derive(Deserialize, Default)]
struct OrPricing {
    #[serde(default)]
    prompt: Amount,
    #[serde(default)]
    completion: Amount,
    #[serde(default)]
    input_cache_read: Amount,
    #[serde(default)]
    input_cache_write: Amount,
}

/// OpenRouter is a gateway that passes provider list prices through. Its
/// `:free` / `:thinking` / `:nitro` … variants share an id with the base model
/// once `normalize_key` drops the suffix, and the free ones are priced 0 —
/// they would overwrite the real row, so any id with a variant tag is skipped.
pub(super) fn import_openrouter(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let root: Loose<OrRoot> = serde_json::from_str(body)?;
    let mut sink = Sink::new(conn, "openrouter", now)?;
    let per_1m = |a: Amount| a.0.map(|x| x * 1e6);
    for m in root.0.unwrap_or_default().data.0 {
        let Some(m) = m.0 else { continue };
        let Some(id) = m.id.0.filter(|id| !id.contains(':')) else {
            continue;
        };
        let p = m.pricing.0.unwrap_or_default();
        sink.put(
            &id,
            per_1m(p.prompt),
            per_1m(p.completion),
            per_1m(p.input_cache_read),
            per_1m(p.input_cache_write),
        )?;
    }
    Ok(sink.n)
}

// ------------------------------------------------------ Vercel AI Gateway

/// `{data:[{id, pricing:{input, output, input_cache_read, input_cache_write}}]}`
/// — decimal strings, $/token. Non-language models carry other shapes, which
/// simply lack `input`/`output` and are skipped.
#[derive(Deserialize, Default)]
struct VcRoot {
    #[serde(default)]
    data: Items<Loose<VcModel>>,
}

#[derive(Deserialize, Default)]
struct VcModel {
    #[serde(default)]
    id: Text,
    #[serde(default)]
    pricing: Loose<VcPricing>,
}

#[derive(Deserialize, Default)]
struct VcPricing {
    #[serde(default)]
    input: Amount,
    #[serde(default)]
    output: Amount,
    #[serde(default)]
    input_cache_read: Amount,
    #[serde(default)]
    input_cache_write: Amount,
}

pub(super) fn import_vercel(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let root: Loose<VcRoot> = serde_json::from_str(body)?;
    let mut sink = Sink::new(conn, "vercel", now)?;
    let per_1m = |a: Amount| a.0.map(|x| x * 1e6);
    for m in root.0.unwrap_or_default().data.0 {
        let Some(m) = m.0 else { continue };
        let Some(id) = m.id.0 else { continue };
        let p = m.pricing.0.unwrap_or_default();
        sink.put(
            &id,
            per_1m(p.input),
            per_1m(p.output),
            per_1m(p.input_cache_read),
            per_1m(p.input_cache_write),
        )?;
    }
    Ok(sink.n)
}

// ---------------------------------------------------------------- Helicone

/// `{data:[{provider, model, operator, input_cost_per_1m, output_cost_per_1m,
/// prompt_cache_read_per_1m, prompt_cache_write_per_1m}]}` — $/1M.
#[derive(Deserialize, Default)]
struct HcRoot {
    #[serde(default)]
    data: Items<Loose<HcRow>>,
}

#[derive(Deserialize, Default)]
struct HcRow {
    #[serde(default)]
    provider: Text,
    #[serde(default)]
    model: Text,
    #[serde(default)]
    operator: Text,
    #[serde(default)]
    input_cost_per_1m: Num,
    #[serde(default)]
    output_cost_per_1m: Num,
    #[serde(default)]
    prompt_cache_read_per_1m: Num,
    #[serde(default)]
    prompt_cache_write_per_1m: Num,
}

/// Helicone's table lists every host of a model, resellers included — its
/// OpenRouter rows carry OpenRouter's 5.5% credit fee (Claude Opus 4.6 at
/// $5.275), its Azure and Together rows are their own prices. Only the model
/// makers' own rows are first-party list prices, so only those are read.
const HELICONE_FIRST_PARTY: &[&str] = &[
    "OPENAI",
    "ANTHROPIC",
    "GOOGLE",
    "MISTRAL",
    "X",
    "DEEPSEEK",
    "COHERE",
    "LLAMA",
    "PERPLEXITY",
];

pub(super) fn import_helicone(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let root: Loose<HcRoot> = serde_json::from_str(body)?;
    let mut sink = Sink::new(conn, "helicone", now)?;
    let mut rows: Vec<HcRow> = root
        .0
        .unwrap_or_default()
        .data
        .0
        .into_iter()
        .filter_map(|r| r.0)
        .filter(|r| {
            r.provider
                .0
                .as_deref()
                .is_some_and(|p| HELICONE_FIRST_PARTY.contains(&p))
        })
        .collect();
    // `equals` rows name one exact model; `startsWith` / `includes` are family
    // patterns. Write patterns first so an exact row of the same id wins the
    // `INSERT OR REPLACE`.
    rows.sort_by_key(|r| r.operator.0.as_deref() == Some("equals"));
    for r in rows {
        let Some(id) = r.model.0 else { continue };
        sink.put(
            &id,
            r.input_cost_per_1m.0,
            r.output_cost_per_1m.0,
            r.prompt_cache_read_per_1m.0,
            r.prompt_cache_write_per_1m.0,
        )?;
    }
    Ok(sink.n)
}

// ---------------------------------------------------------------- Langfuse

/// A JSON array of `{modelName, pricingTiers:[{isDefault, prices:{input,
/// output, input_cache_read, …}}]}` — $/token, maintained by hand in
/// Langfuse's repository.
#[derive(Deserialize, Default)]
struct LfModel {
    #[serde(default, rename = "modelName")]
    model_name: Text,
    #[serde(default, rename = "pricingTiers")]
    tiers: Items<Loose<LfTier>>,
}

#[derive(Deserialize, Default)]
struct LfTier {
    #[serde(default, rename = "isDefault")]
    is_default: Flag,
    #[serde(default)]
    prices: Loose<LfPrices>,
}

/// Only a JSON `true` counts; anything else is `false`.
#[derive(Clone, Copy, Default)]
struct Flag(bool);

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Flag;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Flag, E> {
                Ok(Flag(v))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_str<E>(self, _: &str) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_unit<E>(self) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_none<E>(self) -> Result<Flag, E> {
                Ok(Flag(false))
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Flag, D2::Error> {
                d.deserialize_any(self)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Flag, A::Error> {
                skip_seq(a).map(|()| Flag(false))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Flag, A::Error> {
                skip_map(a).map(|()| Flag(false))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Deserialize, Default)]
struct LfPrices {
    #[serde(default)]
    input: Num,
    #[serde(default)]
    output: Num,
    #[serde(default)]
    input_cache_read: Num,
    #[serde(default)]
    cache_read_input_tokens: Num,
    #[serde(default)]
    input_cached_tokens: Num,
    #[serde(default)]
    input_cache_creation: Num,
    #[serde(default)]
    cache_write_tokens: Num,
    #[serde(default)]
    input_cache_write_tokens: Num,
}

pub(super) fn import_langfuse(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let models: Items<Loose<LfModel>> = serde_json::from_str(body)?;
    let mut sink = Sink::new(conn, "langfuse", now)?;
    let per_1m = |n: Num| n.0.map(|x| x * 1e6);
    for m in models.0 {
        let Some(m) = m.0 else { continue };
        let Some(name) = m.model_name.0 else { continue };
        let tiers: Vec<LfTier> = m.tiers.0.into_iter().filter_map(|t| t.0).collect();
        // The default tier is the standard price; "Fast mode", ">200k" … tiers
        // are surcharges on the same id.
        let default = tiers.iter().position(|t| t.is_default.0).unwrap_or(0);
        let Some(p) = tiers.into_iter().nth(default).and_then(|t| t.prices.0) else {
            continue;
        };
        // Legacy rows price a single blended `total` — no input/output split
        // to vote with.
        if p.input.0.is_none() && p.output.0.is_none() {
            continue;
        }
        let first = |a: Num, b: Num, c: Num| a.0.or(b.0).or(c.0).map(|x| x * 1e6);
        sink.put(
            &name,
            per_1m(p.input),
            per_1m(p.output),
            first(
                p.input_cache_read,
                p.cache_read_input_tokens,
                p.input_cached_tokens,
            ),
            first(
                p.input_cache_creation,
                p.cache_write_tokens,
                p.input_cache_write_tokens,
            ),
        )?;
    }
    Ok(sink.n)
}

// -------------------------------------------------------------- llm-prices

/// `{prices:[{id, name, input, output, input_cached}]}` — $/1M, hand-curated
/// by Simon Willison. Long-context surcharge tiers are separate rows whose
/// display name says `>200k` / `>272k`; they are not the model's base price.
#[derive(Deserialize, Default)]
struct LmRoot {
    #[serde(default)]
    prices: Items<Loose<LmRow>>,
}

#[derive(Deserialize, Default)]
struct LmRow {
    #[serde(default)]
    id: Text,
    #[serde(default)]
    name: Text,
    #[serde(default)]
    input: Num,
    #[serde(default)]
    output: Num,
    #[serde(default)]
    input_cached: Num,
}

pub(super) fn import_llm_prices(conn: &Connection, body: &str, now: i64) -> Result<usize> {
    let root: Loose<LmRoot> = serde_json::from_str(body)?;
    let mut sink = Sink::new(conn, "llm-prices", now)?;
    for r in root.0.unwrap_or_default().prices.0 {
        let Some(r) = r.0 else { continue };
        let Some(id) = r.id.0 else { continue };
        if r.name.0.as_deref().is_some_and(|n| n.contains('>')) {
            continue;
        }
        sink.put(&id, r.input.0, r.output.0, r.input_cached.0, None)?;
    }
    Ok(sink.n)
}

// ----------------------------------------------------------------- Portkey

/// One provider file of Portkey's public model-pricing repository:
/// `{"<model>": {pricing_config: {pay_as_you_go: {request_token:{price},
/// response_token:{price}, cache_read_input_token:{price},
/// cache_write_input_token:{price}}}}, "default": {…}}` — **cents per token**.
#[derive(Deserialize, Default)]
struct PkEntry {
    #[serde(default)]
    pricing_config: Loose<PkConfig>,
}

#[derive(Deserialize, Default)]
struct PkConfig {
    #[serde(default)]
    pay_as_you_go: Loose<PkPayg>,
}

#[derive(Deserialize, Default)]
struct PkPayg {
    #[serde(default)]
    request_token: Loose<PkPrice>,
    #[serde(default)]
    response_token: Loose<PkPrice>,
    #[serde(default)]
    cache_read_input_token: Loose<PkPrice>,
    #[serde(default)]
    cache_write_input_token: Loose<PkPrice>,
}

#[derive(Deserialize, Default)]
struct PkPrice {
    #[serde(default)]
    price: Num,
}

/// Portkey files are per *host*; only the model makers' own files are used
/// (see `PORTKEY_FILES`), and `only_prefixes` narrows a file that also lists
/// other makers' models (DashScope resells Kimi and GLM at its own prices).
pub(super) fn import_portkey(
    conn: &Connection,
    body: &str,
    now: i64,
    only_prefixes: &[&str],
) -> Result<usize> {
    let entries: Entries<Loose<PkEntry>> = serde_json::from_str(body)?;
    let mut sink = Sink::new(conn, "portkey", now)?;
    // cents/token → $/1M.
    let per_1m = |p: Loose<PkPrice>| p.0.and_then(|p| p.price.0).map(|c| c * 1e4);
    for (id, entry) in entries.0 {
        if id == "default"
            || (!only_prefixes.is_empty()
                && !only_prefixes
                    .iter()
                    .any(|p| id.to_ascii_lowercase().starts_with(p)))
        {
            continue;
        }
        let Some(cfg) = entry.0.and_then(|e| e.pricing_config.0) else {
            continue;
        };
        let Some(p) = cfg.pay_as_you_go.0 else {
            continue;
        };
        sink.put(
            &id,
            per_1m(p.request_token),
            per_1m(p.response_token),
            per_1m(p.cache_read_input_token),
            per_1m(p.cache_write_input_token),
        )?;
    }
    Ok(sink.n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use serde_json::{Value, json};

    // -- the original Value-based importers, kept verbatim as the oracle ----

    fn reference_models_dev(conn: &Connection, md: &Value, now: i64) -> Result<usize> {
        let mut st = conn.prepare(
            "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write, source, fetched_at)
             VALUES ('models.dev', ?1, ?2, ?3, ?4, ?5, 'models.dev', ?6)",
        )?;
        let mut n = 0;
        for prov in md.as_object().into_iter().flatten() {
            for (id, m) in prov.1["models"].as_object().into_iter().flatten() {
                let c = &m["cost"];
                if !c.is_object() {
                    continue;
                }
                let f = |k: &str| c[k].as_f64().unwrap_or(0.0);
                st.execute(rusqlite::params![
                    normalize_key(id),
                    f("input"),
                    f("output"),
                    f("cache_read"),
                    f("cache_write"),
                    now
                ])?;
                n += 1;
            }
        }
        Ok(n)
    }

    fn reference_litellm(conn: &Connection, ll: &Value, now: i64) -> Result<usize> {
        let mut st = conn.prepare(
            "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write,
                    tier_above_200k_input, tier_1h_cache_write, tier_batch,
                tier_above_200k_output, tier_above_200k_cache_read, source, fetched_at)
             VALUES ('litellm', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'litellm', ?11)",
        )?;
        let m = |v: &Value, k: &str| v[k].as_f64().map(|x| x * 1e6);
        let mut n = 0;
        for (id, v) in ll.as_object().into_iter().flatten() {
            if !v["input_cost_per_token"].is_number() {
                continue;
            }
            st.execute(rusqlite::params![
                normalize_key(id),
                m(v, "input_cost_per_token").unwrap_or(0.0),
                m(v, "output_cost_per_token").unwrap_or(0.0),
                m(v, "cache_read_input_token_cost").unwrap_or(0.0),
                m(v, "cache_creation_input_token_cost").unwrap_or(0.0),
                m(v, "input_cost_per_token_above_200k_tokens"),
                m(v, "cache_creation_input_token_cost_above_1hr"),
                m(v, "input_cost_per_token_batches"),
                m(v, "output_cost_per_token_above_200k_tokens"),
                m(v, "cache_read_input_token_cost_above_200k_tokens"),
                now
            ])?;
            n += 1;
        }
        Ok(n)
    }

    fn reference_llmpricing(conn: &Connection, lp: &Value, now: i64) -> Result<usize> {
        let mut st = conn.prepare(
            "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write, source, fetched_at)
             VALUES ('llmpricing', ?1, ?2, ?3, ?4, NULL, 'llmpricing', ?5)",
        )?;
        let mut n = 0usize;
        for m in lp["models"].as_array().into_iter().flatten() {
            let Some(id) = m["id"].as_str() else { continue };
            let q = if m["reference"]["input"].is_number() {
                &m["reference"]
            } else {
                &m["cheapest"]
            };
            if !q["input"].is_number() && !q["output"].is_number() {
                continue;
            }
            st.execute(rusqlite::params![
                normalize_key(id),
                q["input"].as_f64().unwrap_or(0.0),
                q["output"].as_f64().unwrap_or(0.0),
                q["cacheRead"].as_f64().unwrap_or(0.0),
                now
            ])?;
            n += 1;
        }
        Ok(n)
    }

    /// Every row of `prices`, in a comparable text form.
    fn dump(s: &Store) -> Vec<String> {
        let mut st = s
            .conn()
            .prepare(
                "SELECT provider||'|'||model_id||'|'||COALESCE(input,'N')||'|'||COALESCE(output,'N')||'|'||
                        COALESCE(cache_read,'N')||'|'||COALESCE(cache_write,'N')||'|'||
                        COALESCE(tier_above_200k_input,'N')||'|'||COALESCE(tier_1h_cache_write,'N')||'|'||
                        COALESCE(tier_batch,'N')||'|'||COALESCE(tier_above_200k_output,'N')||'|'||COALESCE(tier_above_200k_cache_read,'N')||'|'||source||'|'||fetched_at
                 FROM prices WHERE source != 'seed' ORDER BY 1",
            )
            .unwrap();
        st.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    }

    /// Run `typed(body)` and `reference(value)` into two fresh stores and
    /// demand identical counts and identical `prices` rows.
    fn same_rows(
        doc: &Value,
        typed: fn(&Connection, &str, i64) -> Result<usize>,
        reference: fn(&Connection, &Value, i64) -> Result<usize>,
    ) {
        let (a, b) = (Store::open_memory().unwrap(), Store::open_memory().unwrap());
        let body = doc.to_string();
        let na = typed(a.conn(), &body, 42).unwrap();
        let nb = reference(b.conn(), doc, 42).unwrap();
        assert_eq!(na, nb, "row count");
        assert_eq!(dump(&a), dump(&b), "rows for {body}");
    }

    #[test]
    fn models_dev_matches_the_value_implementation() {
        same_rows(
            &json!({
                "openai": {"name": "OpenAI", "models": {
                    "gpt-5": {"cost": {"input": 1.25, "output": 10, "cache_read": 0.125}, "limit": {"context": 400000}, "junk": [1,2,{"a":null}]},
                    "GPT-5-Mini": {"cost": {"input": 0.25, "output": 2, "cache_read": "oops", "cache_write": null}},
                    "no-cost": {"name": "x"},
                    "cost-not-object": {"cost": 5},
                    "cost-array": {"cost": [1,2]},
                    "not-an-object-model": 7,
                    "openai/dup:latest": {"cost": {"input": 3, "output": 4}},
                    "dup": {"cost": {"input": 9, "output": 9}}
                }},
                "weird-provider": {"models": [1, 2, 3]},
                "no-models": {"name": "n"},
                "scalar-provider": 12,
                "null-provider": null,
                "anthropic": {"models": {"claude-x[1m]": {"cost": {"input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75, "extra": {"k": [1]}}}}}
            }),
            import_models_dev,
            reference_models_dev,
        );
        // top-level not an object → nothing, no error
        same_rows(&json!([1, 2]), import_models_dev, reference_models_dev);
        same_rows(&json!("text"), import_models_dev, reference_models_dev);
    }

    #[test]
    fn litellm_matches_the_value_implementation() {
        same_rows(
            &json!({
                "sample_spec": {"max_tokens": "set to max_output_tokens if provider specifies it"},
                "gpt-4o": {"input_cost_per_token": 2.5e-6, "output_cost_per_token": 1e-5, "cache_read_input_token_cost": 1.25e-6,
                           "litellm_provider": "openai", "supports_vision": true},
                "claude-x": {"input_cost_per_token": 3e-6, "output_cost_per_token": 15e-6, "cache_creation_input_token_cost": 3.75e-6,
                             "input_cost_per_token_above_200k_tokens": 6e-6, "cache_creation_input_token_cost_above_1hr": 6e-6,
                             "input_cost_per_token_batches": 1.5e-6,
                             "output_cost_per_token_above_200k_tokens": 22.5e-6, "cache_read_input_token_cost_above_200k_tokens": 0.6e-6},
                "int-cost": {"input_cost_per_token": 1, "output_cost_per_token": 2},
                "string-cost": {"input_cost_per_token": "0.000001"},
                "no-input": {"output_cost_per_token": 1e-6},
                "scalar-entry": 4,
                "array-entry": [1],
                "bedrock/anthropic.claude-y:0": {"input_cost_per_token": 1e-6},
                "claude-y": {"input_cost_per_token": 2e-6}
            }),
            import_litellm,
            reference_litellm,
        );
        same_rows(&json!(null), import_litellm, reference_litellm);
    }

    #[test]
    fn litellm_imports_long_context_output_and_cache_read_tiers() {
        let s = Store::open_memory().unwrap();
        let body =
            json!({"claude-x": {"input_cost_per_token": 3e-6, "output_cost_per_token": 15e-6,
            "output_cost_per_token_above_200k_tokens": 22.5e-6,
            "cache_read_input_token_cost_above_200k_tokens": 0.6e-6}})
            .to_string();
        assert_eq!(import_litellm(s.conn(), &body, 1).unwrap(), 1);
        let (out, cr): (Option<f64>, Option<f64>) = s
            .conn()
            .query_row(
                "SELECT tier_above_200k_output, tier_above_200k_cache_read FROM prices WHERE model_id='claude-x'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!((out.unwrap() - 22.5).abs() < 1e-9);
        assert!((cr.unwrap() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn llmpricing_matches_the_value_implementation() {
        same_rows(
            &json!({"meta": {"models": 6}, "models": [
                {"id": "a/model-a", "reference": {"provider": "p", "input": 1, "output": 2, "cacheRead": 0.1, "official": true},
                 "cheapest": {"input": 0.5, "output": 1}},
                {"id": "model-b", "reference": {"provider": "p"}, "cheapest": {"input": 0.3, "output": 0.9, "cacheRead": 0.03}},
                {"id": "model-c", "reference": {"output": 5}, "cheapest": {"output": 4}},
                {"id": "model-d", "reference": null, "cheapest": {}},
                {"id": 12, "reference": {"input": 1}},
                {"reference": {"input": 1}},
                "junk", 5, null,
                {"id": "model-e", "reference": "str", "cheapest": [1]},
                {"id": "model-f", "reference": {"input": "1"}, "cheapest": {"input": 2, "output": 3}}
            ]}),
            import_llmpricing,
            reference_llmpricing,
        );
        same_rows(
            &json!({"models": {"a": 1}}),
            import_llmpricing,
            reference_llmpricing,
        );
        same_rows(&json!([1]), import_llmpricing, reference_llmpricing);
    }

    /// The real feeds, against the oracle. Needs network-downloaded files, so
    /// it only runs on request:
    /// `GTT_FEED_DIR=<dir with models_dev.json, litellm.json, llmpricing.json>
    ///  cargo test -p globaltokentracker-core real_feeds -- --ignored --nocapture`
    #[test]
    #[ignore = "needs GTT_FEED_DIR with the downloaded feeds"]
    fn real_feeds_match_the_value_implementation() {
        let Some(dir) = std::env::var_os("GTT_FEED_DIR") else {
            eprintln!("GTT_FEED_DIR not set — skipped");
            return;
        };
        type Typed = fn(&Connection, &str, i64) -> Result<usize>;
        type Oracle = fn(&Connection, &Value, i64) -> Result<usize>;
        let cases: [(&str, Typed, Oracle); 3] = [
            ("models_dev.json", import_models_dev, reference_models_dev),
            ("litellm.json", import_litellm, reference_litellm),
            ("llmpricing.json", import_llmpricing, reference_llmpricing),
        ];
        for (file, typed, oracle) in cases {
            let body = std::fs::read_to_string(std::path::Path::new(&dir).join(file)).unwrap();
            let value: Value = serde_json::from_str(&body).unwrap();
            let (a, b) = (Store::open_memory().unwrap(), Store::open_memory().unwrap());
            let na = typed(a.conn(), &body, 7).unwrap();
            let nb = oracle(b.conn(), &value, 7).unwrap();
            assert_eq!(na, nb, "{file}: row count");
            assert_eq!(dump(&a), dump(&b), "{file}: rows differ");
            eprintln!("{file}: {na} rows, identical to the Value implementation");
        }
    }

    // -- the six cross-check feeds ------------------------------------------

    /// `(model_id, input, output, cache_read, cache_write, source)` rows of one
    /// source, model-sorted.
    type Row = (String, f64, f64, Option<f64>, Option<f64>, String);

    fn rows_of(s: &Store, source: &str) -> Vec<Row> {
        let mut st = s
            .conn()
            .prepare(
                "SELECT model_id, input, output, cache_read, cache_write, source
                 FROM prices WHERE source = ?1 ORDER BY model_id",
            )
            .unwrap();
        st.query_map([source], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * a.abs().max(1.0)
    }

    #[test]
    fn openrouter_prices_are_decimal_strings_per_token() {
        let s = Store::open_memory().unwrap();
        let body = json!({"data": [
            {"id": "anthropic/claude-opus-4.6", "pricing": {"prompt": "0.000005", "completion": "0.000025",
                "input_cache_read": "0.0000005", "input_cache_write": "0.00000625", "web_search": "0.01"}},
            // A free variant shares its id with the paid model once ":free" is
            // dropped — it must not overwrite the real row.
            {"id": "deepseek/deepseek-r1:free", "pricing": {"prompt": "0", "completion": "0"}},
            {"id": "deepseek/deepseek-r1", "pricing": {"prompt": "0.0000007", "completion": "0.0000025"}},
            // "-1" marks dynamic-priced routers; no cache fields at all.
            {"id": "openrouter/auto", "pricing": {"prompt": "-1", "completion": "-1"}},
            {"id": "google/gemini-3.8-flash", "pricing": {"prompt": 7.5e-7, "completion": 3.75e-6}},
            {"id": "no-pricing"},
            {"id": "junk", "pricing": "free"},
            {"id": 7, "pricing": {"prompt": "1"}},
            "text", null
        ]})
        .to_string();
        assert_eq!(import_openrouter(s.conn(), &body, 9).unwrap(), 3);
        let rows = rows_of(&s, "openrouter");
        let get = |id: &str| {
            rows.iter()
                .find(|r| r.0 == id)
                .unwrap_or_else(|| panic!("{id}"))
        };
        let o = get("claude-opus-4.6");
        assert!(near(o.1, 5.0) && near(o.2, 25.0), "{o:?}");
        assert!(near(o.3.unwrap(), 0.5) && near(o.4.unwrap(), 6.25), "{o:?}");
        let r1 = get("deepseek-r1");
        assert!(
            near(r1.1, 0.7) && near(r1.2, 2.5),
            "free variant clobbered it: {r1:?}"
        );
        assert!(near(get("gemini-3.8-flash").1, 0.75));
        assert_eq!(get("gemini-3.8-flash").3, None); // absent, not zero
        assert!(rows.iter().all(|r| r.0 != "auto" && r.0 != "junk"));
        assert!(import_openrouter(s.conn(), "not json", 1).is_err());
        assert_eq!(import_openrouter(s.conn(), "[1,2]", 1).unwrap(), 0);
    }

    #[test]
    fn vercel_prices_use_input_output_names() {
        let s = Store::open_memory().unwrap();
        let body = json!({"object": "list", "data": [
            {"id": "anthropic/claude-3-haiku", "type": "language",
             "pricing": {"input": "0.00000025", "output": "0.00000125",
                         "input_cache_read": "0.00000003", "input_cache_write": "0.0000003"}},
            {"id": "openai/text-embedding-3-small", "type": "embedding", "pricing": {"input": "0.00000002"}},
            {"id": "bfl/flux", "type": "image", "pricing": {"per_image": "0.04"}},
            {"id": "x/no-pricing"}
        ]})
        .to_string();
        assert_eq!(import_vercel(s.conn(), &body, 1).unwrap(), 2);
        let rows = rows_of(&s, "vercel");
        assert_eq!(rows.len(), 2);
        let h = rows.iter().find(|r| r.0 == "claude-3-haiku").unwrap();
        assert!(
            near(h.1, 0.25) && near(h.2, 1.25) && near(h.3.unwrap(), 0.03),
            "{h:?}"
        );
        let e = rows
            .iter()
            .find(|r| r.0 == "text-embedding-3-small")
            .unwrap();
        assert!(near(e.1, 0.02) && e.2 == 0.0, "input-only model: {e:?}");
    }

    #[test]
    fn helicone_reads_only_the_model_makers_own_rows() {
        let s = Store::open_memory().unwrap();
        let body = json!({"metadata": {}, "data": [
            {"provider": "ANTHROPIC", "model": "claude-opus-4-6", "operator": "includes",
             "input_cost_per_1m": 5, "output_cost_per_1m": 25,
             "prompt_cache_read_per_1m": 0.5, "prompt_cache_write_per_1m": 6.25},
            // OpenRouter's row carries its 5.5% credit fee — not a list price.
            {"provider": "OPENROUTER", "model": "anthropic/claude-opus-4.6", "operator": "equals",
             "input_cost_per_1m": 5.275, "output_cost_per_1m": 26.375},
            {"provider": "AZURE", "model": "gpt-5", "operator": "equals",
             "input_cost_per_1m": 1.4, "output_cost_per_1m": 11},
            // Same id twice: the exact (`equals`) row beats the family pattern.
            {"provider": "OPENAI", "model": "gpt-5", "operator": "equals",
             "input_cost_per_1m": 1.25, "output_cost_per_1m": 10},
            {"provider": "OPENAI", "model": "gpt-5", "operator": "startsWith",
             "input_cost_per_1m": 9, "output_cost_per_1m": 9},
            {"provider": "OPENAI", "model": "no-prices", "operator": "equals"},
            "junk"
        ]})
        .to_string();
        assert_eq!(import_helicone(s.conn(), &body, 1).unwrap(), 3);
        let rows = rows_of(&s, "helicone");
        let ids: Vec<_> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(ids, ["claude-opus-4-6", "gpt-5"]);
        let g = rows.iter().find(|r| r.0 == "gpt-5").unwrap();
        assert!(near(g.1, 1.25) && near(g.2, 10.0), "exact row lost: {g:?}");
        let o = rows.iter().find(|r| r.0 == "claude-opus-4-6").unwrap();
        assert!(near(o.3.unwrap(), 0.5) && near(o.4.unwrap(), 6.25));
    }

    #[test]
    fn langfuse_uses_the_default_tier_and_skips_blended_rows() {
        let s = Store::open_memory().unwrap();
        let body = json!([
            {"modelName": "gpt-4o", "pricingTiers": [
                {"isDefault": false, "prices": {"input": 5e-6, "output": 2e-5}},
                {"isDefault": true, "prices": {"input": 2.5e-6, "output": 1e-5,
                    "input_cache_read": 1.25e-6, "input_cache_creation": 2.5e-6}}]},
            {"modelName": "claude-x", "pricingTiers": [
                {"isDefault": true, "prices": {"input": 3e-6, "output": 1.5e-5,
                    "cache_read_input_tokens": 3e-7, "cache_write_tokens": 3.75e-6}}]},
            // Legacy blended price: nothing to vote with.
            {"modelName": "text-ada-001", "pricingTiers": [{"isDefault": true, "prices": {"total": 4e-6}}]},
            // No default flag → the first tier.
            {"modelName": "first-tier", "pricingTiers": [{"prices": {"input": 1e-6, "output": 2e-6}}]},
            {"modelName": "no-tiers"},
            {"pricingTiers": []},
            7, null
        ])
        .to_string();
        assert_eq!(import_langfuse(s.conn(), &body, 1).unwrap(), 3);
        let rows = rows_of(&s, "langfuse");
        let g = |id: &str| rows.iter().find(|r| r.0 == id).unwrap();
        let o = g("gpt-4o");
        assert!(
            near(o.1, 2.5)
                && near(o.2, 10.0)
                && near(o.3.unwrap(), 1.25)
                && near(o.4.unwrap(), 2.5)
        );
        let c = g("claude-x");
        assert!(near(c.3.unwrap(), 0.3) && near(c.4.unwrap(), 3.75), "{c:?}");
        assert!(near(g("first-tier").1, 1.0));
        assert!(rows.iter().all(|r| r.0 != "text-ada-001"));
    }

    #[test]
    fn llm_prices_skips_the_long_context_surcharge_rows() {
        let s = Store::open_memory().unwrap();
        let body = json!({"updated_at": "2026-09-28", "prices": [
            {"id": "gpt-5.4", "name": "GPT-5.4 ≤272k", "input": 2.5, "output": 15.0, "input_cached": 0.25},
            {"id": "gpt-5.4-272k", "name": "GPT-5.4 >272k", "input": 5.0, "output": 22.5, "input_cached": 0.5},
            {"id": "gpt-5-nano", "name": "GPT-5 Nano", "input": 0.05, "output": 0.4, "input_cached": null},
            {"id": "no-price", "name": "x"},
            5
        ]})
        .to_string();
        assert_eq!(import_llm_prices(s.conn(), &body, 1).unwrap(), 2);
        let rows = rows_of(&s, "llm-prices");
        assert_eq!(rows.len(), 2);
        let g = rows.iter().find(|r| r.0 == "gpt-5.4").unwrap();
        assert!(near(g.1, 2.5) && near(g.3.unwrap(), 0.25));
        assert_eq!(rows.iter().find(|r| r.0 == "gpt-5-nano").unwrap().3, None);
    }

    #[test]
    fn portkey_prices_are_cents_per_token() {
        let s = Store::open_memory().unwrap();
        let body = json!({
            "default": {"pricing_config": {"pay_as_you_go": {"request_token": {"price": 0}}}},
            "gpt-4o": {"pricing_config": {
                "pay_as_you_go": {"request_token": {"price": 0.00025}, "response_token": {"price": 0.001},
                                  "cache_read_input_token": {"price": 0.000125},
                                  "cache_write_input_token": {"price": 0}},
                "calculate": {"request": {"operation": "sum", "operands": [{"value": "x"}]}}}},
            "qwen3-max": {"pricing_config": {"pay_as_you_go": {"request_token": {"price": 0.00012},
                                                                "response_token": {"price": 0.0006}}}},
            "kimi-k2.5": {"pricing_config": {"pay_as_you_go": {"request_token": {"price": 0.0000574},
                                                                "response_token": {"price": 0.0003011}}}},
            "no-config": {},
            "zero": {"pricing_config": {"pay_as_you_go": {"request_token": {"price": 0}, "response_token": {"price": 0}}}}
        })
        .to_string();
        // Every model of a maker's own file…
        assert_eq!(import_portkey(s.conn(), &body, 1, &[]).unwrap(), 4);
        let rows = rows_of(&s, "portkey");
        let g = |id: &str| rows.iter().find(|r| r.0 == id).unwrap();
        let o = g("gpt-4o");
        assert!(
            near(o.1, 2.5) && near(o.2, 10.0) && near(o.3.unwrap(), 1.25),
            "{o:?}"
        );
        assert!(rows.iter().all(|r| r.0 != "default"));

        // …or only the maker's own prefix out of a reseller's file.
        let s2 = Store::open_memory().unwrap();
        assert_eq!(
            import_portkey(s2.conn(), &body, 1, &["qwen", "qwq"]).unwrap(),
            1
        );
        let only = rows_of(&s2, "portkey");
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].0, "qwen3-max");
        assert!(near(only[0].1, 1.2) && near(only[0].2, 6.0));
        assert!(import_portkey(s2.conn(), "{oops", 1, &[]).is_err());
    }

    /// The real feeds, all nine: each imports rows and agrees with the prices
    /// everyone knows. Needs downloaded files (see the ignored test above):
    /// `GTT_FEED_DIR=<dir with openrouter.json vercel.json helicone.json
    ///  langfuse.json llmprices.json portkey_<provider>.json>`.
    #[test]
    #[ignore = "needs GTT_FEED_DIR with the downloaded feeds"]
    fn real_cross_check_feeds_import_and_agree_on_known_prices() {
        let Some(dir) = std::env::var_os("GTT_FEED_DIR") else {
            eprintln!("GTT_FEED_DIR not set — skipped");
            return;
        };
        let dir = std::path::Path::new(&dir);
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        let s = Store::open_memory().unwrap();
        let mut counts = vec![
            (
                "openrouter",
                import_openrouter(s.conn(), &read("openrouter.json"), 1).unwrap(),
            ),
            (
                "vercel",
                import_vercel(s.conn(), &read("vercel.json"), 1).unwrap(),
            ),
            (
                "helicone",
                import_helicone(s.conn(), &read("helicone.json"), 1).unwrap(),
            ),
            (
                "langfuse",
                import_langfuse(s.conn(), &read("langfuse.json"), 1).unwrap(),
            ),
            (
                "llm-prices",
                import_llm_prices(s.conn(), &read("llmprices.json"), 1).unwrap(),
            ),
        ];
        let mut portkey = 0;
        for (p, prefixes) in [
            ("anthropic", &[][..]),
            ("openai", &[][..]),
            ("google", &[][..]),
            ("x-ai", &[][..]),
            ("dashscope", &["qwen", "qwq"][..]),
        ] {
            if let Ok(body) = std::fs::read_to_string(dir.join(format!("portkey_{p}.json"))) {
                portkey += import_portkey(s.conn(), &body, 1, prefixes).unwrap();
            }
        }
        counts.push(("portkey", portkey));
        for (src, n) in &counts {
            eprintln!("{src}: {n} rows");
            assert!(
                *n > 100 || *src == "llm-prices" || *src == "langfuse",
                "{src}: only {n}"
            );
            assert!(*n > 0, "{src} imported nothing");
        }
        // Everyone that knows Claude Opus 4.6 quotes $5 / $25.
        let opus: Vec<(String, f64, f64)> =
            ["openrouter", "vercel", "langfuse", "llm-prices", "portkey"]
                .iter()
                .filter_map(|src| {
                    rows_of(&s, src)
                        .into_iter()
                        .find(|r| r.0.replace('.', "-") == "claude-opus-4-6")
                        .map(|r| (src.to_string(), r.1, r.2))
                })
                .collect();
        eprintln!("claude-opus-4-6: {opus:?}");
        assert!(opus.len() >= 4, "{opus:?}");
        assert!(
            opus.iter().all(|(_, i, o)| near(*i, 5.0) && near(*o, 25.0)),
            "{opus:?}"
        );
    }

    #[test]
    fn malformed_json_is_an_error_not_a_partial_import() {
        let s = Store::open_memory().unwrap();
        assert!(import_models_dev(s.conn(), "{\"a\": {\"models\": ", 1).is_err());
        assert!(import_litellm(s.conn(), "not json", 1).is_err());
        assert_eq!(dump(&s).len(), 0);
    }
}
