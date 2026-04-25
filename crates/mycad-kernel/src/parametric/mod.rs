//! Parametric framework: history DAG, component tree, feature trait, rebuild engine.
//!
//! Every operation that mutates the document is a `HistoryNode` whose `operation`
//! payload implements [`feature::Feature`]. A [`document::Document`] owns the
//! DAG, a set of [`types::Component`]s, and a set of [`types::Branch`]es.
//! Edits are re-solved by [`rebuild::rebuild`], which resolves cross-node
//! references through [`naming::SignatureResolver`].

pub mod document;
pub mod errors;
pub mod feature;
pub mod naming;
pub mod ops;
pub mod rebuild;
pub mod types;
