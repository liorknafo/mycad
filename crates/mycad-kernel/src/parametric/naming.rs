//! Topological naming: C-minimal signature matcher.
//!
//! Signatures are computed for B-Rep entities when a feature produces them,
//! and stored alongside the entity's `BRepId`. At rebuild time, downstream
//! features reference entities via stored signatures; the `SignatureResolver`
//! matches old signatures to current entity IDs in the producing node's
//! output using a deterministic, quantized, two-stage algorithm.

use crate::brep::BRepId;
use crate::math::{Scalar, Vec3};
use crate::parametric::errors::ParametricError;
use serde::{Deserialize, Serialize};

/// Signature format version. Bumped when the layout of [`EntitySignature`] changes.
pub const CURRENT_SIGNATURE_VERSION: u32 = 1;

/// Kind of B-Rep entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityKind {
    Vertex,
    Edge,
    Face,
}

/// Geometric type of a B-Rep entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeometryKind {
    Point,
    Line,
    Circle,
    Arc,
    Plane,
    Cylinder,
}

/// Quantized 3D point (value * 1000, rounded). No floats, so comparison is exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuantizedPoint3 {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

impl QuantizedPoint3 {
    /// Quantize a 3D point to the 1e-3 mm grid.
    pub fn from_point(p: Vec3) -> Self {
        Self {
            x: (p.x * 1000.0).round() as i64,
            y: (p.y * 1000.0).round() as i64,
            z: (p.z * 1000.0).round() as i64,
        }
    }

    /// Manhattan distance between two quantized points.
    pub fn manhattan(a: Self, b: Self) -> i64 {
        (a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs()
    }
}

/// Quantized scalar (value * 1000, rounded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuantizedScalar(pub i64);

impl QuantizedScalar {
    pub fn from_scalar(s: Scalar) -> Self {
        Self((s * 1000.0).round() as i64)
    }
}

/// A signature for a B-Rep entity. Computed at feature-build time and used
/// later to resolve `InputRef::BRepEntity` references across rebuilds.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntitySignature {
    pub version: u32,
    pub kind: EntityKind,
    pub geometry: GeometryKind,
    pub adjacency: Vec<GeometryKind>,
    pub centroid: QuantizedPoint3,
    pub measure: QuantizedScalar,
    pub sibling_rank: u32,
}

/// Errors returned from [`SignatureResolver::resolve_brep_entity`].
#[derive(Debug, Clone)]
pub enum ResolveError {
    NoMatch,
    AmbiguousExact,
    Ambiguous,
    ProducingNodeMissing,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMatch => write!(f, "no matching candidate"),
            Self::AmbiguousExact => write!(f, "multiple exact matches"),
            Self::Ambiguous => write!(f, "multiple nearest candidates"),
            Self::ProducingNodeMissing => write!(f, "producing node missing"),
        }
    }
}

impl From<ResolveError> for ParametricError {
    fn from(e: ResolveError) -> Self {
        ParametricError::Other(e.to_string())
    }
}

/// Resolves an `InputRef::BRepEntity` to a current `BRepId` in the producing
/// node's output, using stored signatures.
pub trait SignatureResolver {
    fn resolve_brep_entity(
        &self,
        producing_node_output: &crate::parametric::feature::FeatureOutput,
        stored_signature: &EntitySignature,
    ) -> Result<BRepId, ResolveError>;
}

/// Default implementation of signature resolution. Used by the rebuild engine.
/// See spec Section 3 for the algorithm.
pub fn default_resolve(
    output: &crate::parametric::feature::FeatureOutput,
    stored: &EntitySignature,
) -> Result<BRepId, ResolveError> {
    // Stage 1: exact match on (kind, geometry, adjacency, sibling_rank).
    let stage1: Vec<&crate::parametric::feature::ProducedEntity> = output
        .produced
        .iter()
        .filter(|p| {
            p.signature.kind == stored.kind
                && p.signature.geometry == stored.geometry
                && p.signature.adjacency == stored.adjacency
                && p.signature.sibling_rank == stored.sibling_rank
        })
        .collect();

    match stage1.len() {
        1 => return Ok(stage1[0].brep_id),
        0 => {}
        _ => return Err(ResolveError::AmbiguousExact),
    }

    // Stage 2: relax sibling_rank, pick nearest by centroid.
    let stage2: Vec<&crate::parametric::feature::ProducedEntity> = output
        .produced
        .iter()
        .filter(|p| {
            p.signature.kind == stored.kind
                && p.signature.geometry == stored.geometry
                && p.signature.adjacency == stored.adjacency
        })
        .collect();

    if stage2.is_empty() {
        return Err(ResolveError::NoMatch);
    }

    let best = stage2
        .iter()
        .min_by_key(|p| QuantizedPoint3::manhattan(p.signature.centroid, stored.centroid))
        .copied()
        .unwrap();

    // Tie-breaker: if more than one candidate is within 1 quantum of best, refuse.
    let best_distance = QuantizedPoint3::manhattan(best.signature.centroid, stored.centroid);
    let tie_count = stage2
        .iter()
        .filter(|p| {
            QuantizedPoint3::manhattan(p.signature.centroid, stored.centroid) - best_distance < 1
        })
        .count();

    if tie_count > 1 {
        return Err(ResolveError::Ambiguous);
    }

    Ok(best.brep_id)
}

/// Zero-state resolver that delegates to `default_resolve`.
pub struct DefaultResolver;

impl SignatureResolver for DefaultResolver {
    fn resolve_brep_entity(
        &self,
        output: &crate::parametric::feature::FeatureOutput,
        stored: &EntitySignature,
    ) -> Result<BRepId, ResolveError> {
        default_resolve(output, stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brep::BRepId;
    use crate::parametric::feature::{FeatureOutput, ProducedEntity};

    #[test]
    fn quantized_point_rounds_to_1e3_grid() {
        // 1.2345 * 1000 = 1234.5, which rounds away from zero to 1235.
        let p = Vec3::new(1.2345, -0.0009, 3.0);
        let q = QuantizedPoint3::from_point(p);
        assert_eq!(q.x, 1235);
        assert_eq!(q.y, -1);
        assert_eq!(q.z, 3000);
    }

    #[test]
    fn quantized_point_equal_for_noise() {
        let a = QuantizedPoint3::from_point(Vec3::new(1.0000001, 2.0, 3.0));
        let b = QuantizedPoint3::from_point(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(a, b);
    }

    #[test]
    fn quantized_scalar_rounds() {
        let a = QuantizedScalar::from_scalar(5.0);
        let b = QuantizedScalar::from_scalar(5.0004);
        assert_eq!(a, b);
    }

    #[test]
    fn manhattan_distance_zero_for_equal_points() {
        let a = QuantizedPoint3 { x: 1, y: 2, z: 3 };
        assert_eq!(QuantizedPoint3::manhattan(a, a), 0);
    }

    #[test]
    fn manhattan_distance_sums_axes() {
        let a = QuantizedPoint3 { x: 0, y: 0, z: 0 };
        let b = QuantizedPoint3 { x: 1, y: 2, z: -3 };
        assert_eq!(QuantizedPoint3::manhattan(a, b), 6);
    }

    fn sig(
        kind: EntityKind,
        geom: GeometryKind,
        adj: &[GeometryKind],
        centroid: QuantizedPoint3,
        measure: i64,
        rank: u32,
    ) -> EntitySignature {
        EntitySignature {
            version: CURRENT_SIGNATURE_VERSION,
            kind,
            geometry: geom,
            adjacency: adj.to_vec(),
            centroid,
            measure: QuantizedScalar(measure),
            sibling_rank: rank,
        }
    }

    fn make_output_with(entities: Vec<(u64, EntitySignature)>) -> FeatureOutput {
        let mut out = FeatureOutput::empty();
        for (id, signature) in entities {
            out.produced.push(ProducedEntity {
                brep_id: BRepId(id),
                signature,
            });
        }
        out
    }

    #[test]
    fn resolver_exact_match_returns_id() {
        let s = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane, GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        let out = make_output_with(vec![(42, s.clone())]);
        let resolver = DefaultResolver;
        let id = resolver.resolve_brep_entity(&out, &s).unwrap();
        assert_eq!(id, BRepId(42));
    }

    #[test]
    fn resolver_no_match_errors() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        let out = make_output_with(vec![]);
        let resolver = DefaultResolver;
        let err = resolver.resolve_brep_entity(&out, &stored).unwrap_err();
        assert!(matches!(err, ResolveError::NoMatch));
    }

    #[test]
    fn resolver_nearest_centroid_picks_closest() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        let c1 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5100 },
            25_000,
            1,
        );
        let c2 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 9000 },
            25_000,
            2,
        );
        let out = make_output_with(vec![(7, c1), (8, c2)]);
        let resolver = DefaultResolver;
        let id = resolver.resolve_brep_entity(&out, &stored).unwrap();
        assert_eq!(id, BRepId(7));
    }

    #[test]
    fn resolver_ambiguous_centroid_errors() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        let c1 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5100 },
            25_000,
            1,
        );
        let c2 = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5100 },
            25_000,
            2,
        );
        let out = make_output_with(vec![(7, c1), (8, c2)]);
        let resolver = DefaultResolver;
        let err = resolver.resolve_brep_entity(&out, &stored).unwrap_err();
        assert!(matches!(err, ResolveError::Ambiguous));
    }

    #[test]
    fn resolver_exact_ambiguous_errors() {
        let stored = sig(
            EntityKind::Face,
            GeometryKind::Plane,
            &[GeometryKind::Plane],
            QuantizedPoint3 { x: 0, y: 0, z: 5000 },
            25_000,
            0,
        );
        let out = make_output_with(vec![(7, stored.clone()), (8, stored.clone())]);
        let resolver = DefaultResolver;
        let err = resolver.resolve_brep_entity(&out, &stored).unwrap_err();
        assert!(matches!(err, ResolveError::AmbiguousExact));
    }
}
