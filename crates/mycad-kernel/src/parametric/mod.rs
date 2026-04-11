//! Parametric framework: history DAG, component tree, feature trait, rebuild engine.
//!
//! Every operation that mutates the document is a `HistoryNode` whose `operation`
//! payload implements [`feature::Feature`]. A [`document::REPLACE_ME`] owns the
//! DAG, a set of [`types::Component`]s, and a set of [`types::Branch`]es.
//! Edits are re-solved by [`rebuild::RebuildEngine`], which resolves cross-node
//! references through [`naming::SignatureResolver`].

pub mod errors;
pub mod feature;
pub mod naming;
pub mod types;
pub mod document;
pub mod rebuild;
