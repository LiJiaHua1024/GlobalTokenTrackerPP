//! codeledger-core — cross-platform usage-tracking engine.
//!
//! Contains zero platform/UI dependencies: every adapter, the normalization
//! pipeline, pricing, SQLite storage and the sync scheduler live here so the
//! Windows (WinUI 3) and future macOS shells stay thin view layers.

pub mod adapters;
pub mod engine;
pub mod model;
pub mod normalize;
pub mod pricing;
pub mod store;
pub mod sync;

pub use engine::{Engine, ScanReport};
pub use model::{CostSource, Provenance, QuotaSnapshot, UsageEvent, apps};
pub use store::Store;
