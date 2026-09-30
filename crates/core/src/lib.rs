//! globaltokentracker-core — cross-platform usage-tracking engine.
//!
//! Contains zero platform/UI dependencies: every adapter, the normalization
//! pipeline, pricing, SQLite storage and the sync scheduler live here so the
//! Windows (WinUI 3) and future macOS shells stay thin view layers.

pub mod adapters;
pub mod cube;
pub mod engine;
pub mod model;
pub mod normalize;
pub mod otel;
pub mod power;
pub mod pricing;
pub mod quota;
pub mod store;
pub mod sync;
pub mod viewmodel;

pub use cube::Cube;
pub use engine::{Engine, ScanReport};
pub use model::{CostSource, Provenance, QuotaSnapshot, UsageEvent, apps};
pub use store::Store;
pub use viewmodel::{DetailVm, OverviewVm};
