//! 2D sketch entities, constraints, and a minimal sketch solver.

use std::ops::RangeInclusive;

use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};

use crate::math::{nearly_zero, Plane, Point2, Point3, Scalar, Vec2, EPSILON};

const SOLVER_DEFAULT_MAX_ITERATIONS: usize = 32;
const SOLVER_DEFAULT_TOLERANCE: Scalar = 1.0e-8;
const SOLVER_NUMERICAL_EPSILON: Scalar = 1.0e-6;
const SOLVER_INITIAL_DAMPING: Scalar = 1.0e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SketchEntityId(pub u64);

/// An ordered closed loop of sketch entities forming a wire boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireLoop {
    pub edges: Vec<SketchEntityId>,
}

/// Errors that can occur during wire extraction from a sketch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireExtractionError {
    NoClosedLoop,
    DisconnectedEdges,
    MixedGeometryTypes,
    OpenLoop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SketchConstraintId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SketchPlaneRef {
    WorldXY,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityPointKind {
    Position,
    Start,
    End,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityPointRef {
    pub entity: SketchEntityId,
    pub kind: EntityPointKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeometryRef {
    Entity(SketchEntityId),
    Point(EntityPointRef),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub plane_ref: SketchPlaneRef,
    pub plane: Plane,
    pub next_id: u64,
    pub next_constraint_id: u64,
    pub entities: Vec<SketchEntity>,
    pub constraints: Vec<SketchConstraint>,
}

impl Sketch {
    pub fn new(plane: Plane) -> Self {
        Self {
            plane_ref: SketchPlaneRef::Custom,
            plane,
            next_id: 1,
            next_constraint_id: 1,
            entities: Vec::new(),
            constraints: Vec::new(),
        }
    }

    pub fn world_xy() -> Self {
        Self {
            plane_ref: SketchPlaneRef::WorldXY,
            plane: Plane::world_xy(),
            next_id: 1,
            next_constraint_id: 1,
            entities: Vec::new(),
            constraints: Vec::new(),
        }
    }

    pub fn add_entity(&mut self, geometry: SketchGeometry, construction: bool, name: Option<String>) -> SketchEntityId {
        let id = SketchEntityId(self.next_id);
        self.next_id += 1;
        self.entities.push(SketchEntity {
            id,
            construction,
            name,
            geometry,
        });
        id
    }

    pub fn remove_entity(&mut self, id: SketchEntityId) -> Option<SketchEntity> {
        let index = self.entities.iter().position(|entity| entity.id == id)?;
        Some(self.entities.remove(index))
    }

    pub fn entity(&self, id: SketchEntityId) -> Option<&SketchEntity> {
        self.entities.iter().find(|entity| entity.id == id)
    }

    pub fn entity_mut(&mut self, id: SketchEntityId) -> Option<&mut SketchEntity> {
        self.entities.iter_mut().find(|entity| entity.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SketchEntity> {
        self.entities.iter()
    }

    pub fn add_constraint(&mut self, kind: SketchConstraintKind, name: Option<String>) -> SketchConstraintId {
        let id = SketchConstraintId(self.next_constraint_id);
        self.next_constraint_id += 1;
        self.constraints.push(SketchConstraint {
            id,
            name,
            enabled: true,
            kind,
        });
        id
    }

    pub fn remove_constraint(&mut self, id: SketchConstraintId) -> Option<SketchConstraint> {
        let index = self.constraints.iter().position(|constraint| constraint.id == id)?;
        Some(self.constraints.remove(index))
    }

    pub fn constraint(&self, id: SketchConstraintId) -> Option<&SketchConstraint> {
        self.constraints.iter().find(|constraint| constraint.id == id)
    }

    pub fn constraint_mut(&mut self, id: SketchConstraintId) -> Option<&mut SketchConstraint> {
        self.constraints.iter_mut().find(|constraint| constraint.id == id)
    }

    pub fn constraints(&self) -> impl Iterator<Item = &SketchConstraint> {
        self.constraints.iter()
    }

    pub fn resolve_geometry_ref(&self, reference: GeometryRef) -> Option<ResolvedGeometryRef<'_>> {
        match reference {
            GeometryRef::Entity(id) => self.entity(id).map(ResolvedGeometryRef::Entity),
            GeometryRef::Point(point_ref) => self.resolve_point_ref(point_ref).map(|point| ResolvedGeometryRef::Point {
                point_ref,
                position: point,
            }),
        }
    }

    pub fn resolve_point_ref(&self, point_ref: EntityPointRef) -> Option<Point2> {
        self.entity(point_ref.entity)?.geometry.point(point_ref.kind)
    }

    pub fn local_point_to_world(&self, point: Point2) -> Point3 {
        self.plane.point_from_uv(point)
    }

    pub fn world_point_to_local(&self, point: Point3) -> Point2 {
        self.plane.project_point(point)
    }

    /// Create a rectangle from two corner points.
    /// Returns the IDs of the 4 created line segments (bottom, right, top, left).
    /// Automatically adds horizontal constraints to top/bottom edges and
    /// vertical constraints to left/right edges, then solves.
    pub fn add_rectangle(&mut self, corner1: Point2, corner2: Point2) -> [SketchEntityId; 4] {
        let min_x = corner1.x.min(corner2.x);
        let min_y = corner1.y.min(corner2.y);
        let max_x = corner1.x.max(corner2.x);
        let max_y = corner1.y.max(corner2.y);
        
        let p1 = Point2::new(min_x, min_y);
        let p2 = Point2::new(max_x, min_y);
        let p3 = Point2::new(max_x, max_y);
        let p4 = Point2::new(min_x, max_y);
        
        // Create 4 line segments: bottom, right, top, left
        let bottom = self.add_entity(
            SketchGeometry::LineSegment(LineSegment { start: p1, end: p2 }),
            false,
            None,
        );
        let right = self.add_entity(
            SketchGeometry::LineSegment(LineSegment { start: p2, end: p3 }),
            false,
            None,
        );
        let top = self.add_entity(
            SketchGeometry::LineSegment(LineSegment { start: p3, end: p4 }),
            false,
            None,
        );
        let left = self.add_entity(
            SketchGeometry::LineSegment(LineSegment { start: p4, end: p1 }),
            false,
            None,
        );
        
        // Add coincident constraints at shared corners to keep them connected
        self.add_constraint(SketchConstraintKind::Coincident {
            a: EntityPointRef { entity: bottom, kind: EntityPointKind::End },
            b: EntityPointRef { entity: right, kind: EntityPointKind::Start },
        }, Some("corner BR".to_string()));
        self.add_constraint(SketchConstraintKind::Coincident {
            a: EntityPointRef { entity: right, kind: EntityPointKind::End },
            b: EntityPointRef { entity: top, kind: EntityPointKind::Start },
        }, Some("corner TR".to_string()));
        self.add_constraint(SketchConstraintKind::Coincident {
            a: EntityPointRef { entity: top, kind: EntityPointKind::End },
            b: EntityPointRef { entity: left, kind: EntityPointKind::Start },
        }, Some("corner TL".to_string()));
        self.add_constraint(SketchConstraintKind::Coincident {
            a: EntityPointRef { entity: left, kind: EntityPointKind::End },
            b: EntityPointRef { entity: bottom, kind: EntityPointKind::Start },
        }, Some("corner BL".to_string()));
        
        // Add horizontal constraints to bottom and top edges
        self.add_constraint(SketchConstraintKind::Horizontal { line: bottom }, None);
        self.add_constraint(SketchConstraintKind::Horizontal { line: top }, None);
        
        // Add vertical constraints to right and left edges
        self.add_constraint(SketchConstraintKind::Vertical { line: right }, None);
        self.add_constraint(SketchConstraintKind::Vertical { line: left }, None);
        
        // Solve to apply constraints
        self.solve();
        
        [bottom, right, top, left]
    }

    /// Extract a closed wire loop from the sketch's line segments.
    /// Returns an ordered list of edge IDs forming a closed boundary.
    pub fn extract_closed_wire(&self) -> Result<WireLoop, WireExtractionError> {
        // Collect all LineSegment entities
        let line_entities: Vec<(SketchEntityId, &LineSegment)> = self.entities
            .iter()
            .filter_map(|e| match &e.geometry {
                SketchGeometry::LineSegment(line) => Some((e.id, line)),
                _ => None,
            })
            .collect();

        if line_entities.is_empty() {
            return Err(WireExtractionError::NoClosedLoop);
        }

        // Build adjacency graph: for each line, find connected lines
        // Two lines are connected if one's endpoint matches another's within tolerance
        let n = line_entities.len();
        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut id_to_index: std::collections::HashMap<SketchEntityId, usize> = std::collections::HashMap::new();
        
        for (i, (id, _)) in line_entities.iter().enumerate() {
            id_to_index.insert(*id, i);
        }

        // Helper to check if two points match within tolerance
        // Use a larger tolerance than global EPSILON because after constraint solving,
        // shared points may differ slightly due to floating point adjustments.
        let points_match = |a: Point2, b: Point2| (a - b).length() < 1.0e-3;

        // Build connections based on shared endpoints
        for (i, (_, line_i)) in line_entities.iter().enumerate() {
            for (j, (_, line_j)) in line_entities.iter().enumerate() {
                if i == j {
                    continue;
                }
                // Check if any endpoint of line_i matches any endpoint of line_j
                if points_match(line_i.start, line_j.start) ||
                   points_match(line_i.start, line_j.end) ||
                   points_match(line_i.end, line_j.start) ||
                   points_match(line_i.end, line_j.end) {
                    adjacency[i].push(j);
                }
            }
        }

        // Check for multiple components using DFS
        let mut visited = vec![false; n];
        let mut component_count = 0;
        let mut component_start = 0;
        
        for i in 0..n {
            if !visited[i] {
                component_count += 1;
                if component_count > 1 {
                    return Err(WireExtractionError::DisconnectedEdges);
                }
                component_start = i;
                // DFS
                let mut stack = vec![i];
                while let Some(node) = stack.pop() {
                    if visited[node] {
                        continue;
                    }
                    visited[node] = true;
                    for &neighbor in &adjacency[node] {
                        if !visited[neighbor] {
                            stack.push(neighbor);
                        }
                    }
                }
            }
        }

        // Traverse to find a cycle
        // Start from the first line, follow connected lines until we return to start
        let mut path: Vec<usize> = vec![component_start];
        let mut current = component_start;
        let mut prev_endpoint: Option<Point2> = None;
        
        loop {
            let (_, current_line) = line_entities[current];
            let next_end = if let Some(prev) = prev_endpoint {
                // Find which end we came from, go to the other end
                if points_match(prev, current_line.start) {
                    current_line.end
                } else {
                    current_line.start
                }
            } else {
                // First line, pick start as our reference
                current_line.end
            };

            // Find a neighbor that connects to next_end
            let mut found_next = false;
            for &neighbor in &adjacency[current] {
                if path.len() > 1 && neighbor == path[path.len() - 2] {
                    // Don't go back immediately
                    continue;
                }
                let (_, neighbor_line) = line_entities[neighbor];
                if points_match(next_end, neighbor_line.start) ||
                   points_match(next_end, neighbor_line.end) {
                    // Check if completing a cycle
                    if neighbor == component_start && path.len() >= 3 {
                        // Check if this actually closes the loop
                        let (_, first_line) = line_entities[component_start];
                        let last_line_end = if points_match(next_end, neighbor_line.start) {
                            neighbor_line.end
                        } else {
                            neighbor_line.start
                        };
                        if points_match(last_line_end, first_line.start) ||
                           points_match(last_line_end, first_line.end) {
                            // Don't push the start node again - the path is already closed
                            // (the last edge connects back to the first edge)
                            let edges: Vec<SketchEntityId> = path
                                .iter()
                                .map(|&i| line_entities[i].0)
                                .collect();
                            return Ok(WireLoop { edges });
                        }
                    }
                    if !path.contains(&neighbor) {
                        path.push(neighbor);
                        prev_endpoint = Some(next_end);
                        current = neighbor;
                        found_next = true;
                        break;
                    }
                }
            }

            if !found_next {
                break;
            }

            // Safety check for infinite loop
            if path.len() > n {
                break;
            }
        }

        // If we get here, we didn't find a closed loop
        Err(WireExtractionError::OpenLoop)
    }

    /// Compute visual positions and states for all constraints for UI rendering.
    /// Call this after solve() to get current constraint states.
    pub fn get_constraint_visuals(&self, solve_result: &SketchSolveResult) -> Vec<ConstraintVisual> {
        let mut visuals = Vec::new();
        
        for constraint in &self.constraints {
            if !constraint.enabled {
                continue;
            }
            
            // Determine state based on solve result
            let state = if solve_result.error.is_some() {
                ConstraintState::Conflicting
            } else if solve_result.status == SketchSolveStatus::Converged {
                ConstraintState::Satisfied
            } else {
                ConstraintState::Normal
            };
            
            // Compute display position based on constraint type
            let display_position = match &constraint.kind {
                SketchConstraintKind::Coincident { a, b } => {
                    // Display at midpoint between the two points
                    if let (Some(pa), Some(pb)) = (self.resolve_point_ref(*a), self.resolve_point_ref(*b)) {
                        (pa + pb) * 0.5
                    } else {
                        Point2::ZERO
                    }
                }
                SketchConstraintKind::Horizontal { line } | SketchConstraintKind::Vertical { line } => {
                    // Display at line midpoint
                    if let Some(entity) = self.entity(*line) {
                        if let SketchGeometry::LineSegment(line_seg) = &entity.geometry {
                            (line_seg.start + line_seg.end) * 0.5
                        } else {
                            Point2::ZERO
                        }
                    } else {
                        Point2::ZERO
                    }
                }
                SketchConstraintKind::Distance { a, b, .. } => {
                    // Display at midpoint between the two points
                    if let (Some(pa), Some(pb)) = (self.resolve_point_ref(*a), self.resolve_point_ref(*b)) {
                        (pa + pb) * 0.5
                    } else {
                        Point2::ZERO
                    }
                }
            };
            
            visuals.push(ConstraintVisual {
                constraint_id: constraint.id,
                kind: constraint.kind,
                display_position,
                state,
            });
        }
        
        visuals
    }

    pub fn solve(&mut self) -> SketchSolveResult {
        let mut parameters = match SketchParameters::from_sketch(self) {
            Ok(parameters) => parameters,
            Err(error) => {
                return SketchSolveResult {
                    status: SketchSolveStatus::Failed,
                    iterations: 0,
                    residual_norm: 0.0,
                    dof: None,
                    error: Some(error),
                };
            }
        };

        let mut residual = match evaluate_residuals(self, &parameters.values) {
            Ok(residual) => residual,
            Err(error) => {
                return SketchSolveResult {
                    status: SketchSolveStatus::Failed,
                    iterations: 0,
                    residual_norm: 0.0,
                    dof: None,
                    error: Some(error),
                };
            }
        };

        let mut residual_norm = residual.norm();
        if residual_norm <= SOLVER_DEFAULT_TOLERANCE {
            return SketchSolveResult {
                status: SketchSolveStatus::Converged,
                iterations: 0,
                residual_norm,
                dof: Some(parameters.values.len().saturating_sub(residual.len())),
                error: None,
            };
        }

        let mut damping = SOLVER_INITIAL_DAMPING;
        let mut performed_iterations = 0;

        for iteration in 0..SOLVER_DEFAULT_MAX_ITERATIONS {
            let jacobian = match numerical_jacobian(self, &parameters.values, &residual) {
                Ok(jacobian) => jacobian,
                Err(error) => {
                    return SketchSolveResult {
                        status: SketchSolveStatus::Failed,
                        iterations: iteration,
                        residual_norm,
                        dof: None,
                        error: Some(error),
                    };
                }
            };

            let jt = jacobian.transpose();
            let identity = DMatrix::<Scalar>::identity(parameters.values.len(), parameters.values.len());
            let lhs = (&jt * &jacobian) + identity.scale(damping);
            let rhs = -(&jt * &residual);

            let Some(delta) = lhs.lu().solve(&rhs) else {
                return SketchSolveResult {
                    status: SketchSolveStatus::Failed,
                    iterations: iteration,
                    residual_norm,
                    dof: None,
                    error: Some(SketchSolveError::SingularSystem),
                };
            };

            if delta.norm() <= SOLVER_DEFAULT_TOLERANCE {
                parameters.apply_to_sketch(self);
                return SketchSolveResult {
                    status: SketchSolveStatus::Converged,
                    iterations: iteration,
                    residual_norm,
                    dof: Some(parameters.values.len().saturating_sub(residual.len())),
                    error: None,
                };
            }

            let candidate = &parameters.values + &delta;
            let candidate_residual = match evaluate_residuals(self, &candidate) {
                Ok(candidate_residual) => candidate_residual,
                Err(error) => {
                    return SketchSolveResult {
                        status: SketchSolveStatus::Failed,
                        iterations: iteration,
                        residual_norm,
                        dof: None,
                        error: Some(error),
                    };
                }
            };
            let candidate_norm = candidate_residual.norm();

            if candidate_norm < residual_norm {
                parameters.values = candidate;
                residual = candidate_residual;
                residual_norm = candidate_norm;
                damping = (damping * 0.5).max(1.0e-9);
                performed_iterations = iteration + 1;

                if residual_norm <= SOLVER_DEFAULT_TOLERANCE {
                    parameters.apply_to_sketch(self);
                    return SketchSolveResult {
                        status: SketchSolveStatus::Converged,
                        iterations: performed_iterations,
                        residual_norm,
                        dof: Some(parameters.values.len().saturating_sub(residual.len())),
                        error: None,
                    };
                }
            } else {
                damping = (damping * 10.0).min(1.0e12);
            }
        }

        parameters.apply_to_sketch(self);
        SketchSolveResult {
            status: SketchSolveStatus::MaxIterationsReached,
            iterations: performed_iterations.max(SOLVER_DEFAULT_MAX_ITERATIONS),
            residual_norm,
            dof: Some(parameters.values.len().saturating_sub(residual.len())),
            error: Some(SketchSolveError::DidNotConverge),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchConstraint {
    pub id: SketchConstraintId,
    pub name: Option<String>,
    pub enabled: bool,
    pub kind: SketchConstraintKind,
}

/// Visual representation data for rendering a constraint in the UI
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConstraintVisual {
    pub constraint_id: SketchConstraintId,
    pub kind: SketchConstraintKind,
    /// Screen/display position for constraint icon/label
    pub display_position: Point2,
    /// Color indicator: 0=normal, 1=satisfied, 2=conflicting, 3=over-constrained
    pub state: ConstraintState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintState {
    Normal,
    Satisfied,
    Conflicting,
    OverConstrained,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedGeometryRef<'a> {
    Entity(&'a SketchEntity),
    Point { point_ref: EntityPointRef, position: Point2 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SketchConstraintKind {
    Coincident { a: EntityPointRef, b: EntityPointRef },
    Horizontal { line: SketchEntityId },
    Vertical { line: SketchEntityId },
    Distance { a: EntityPointRef, b: EntityPointRef, distance: Scalar },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SketchSolveStatus {
    Converged,
    MaxIterationsReached,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SketchSolveError {
    MissingEntity(SketchEntityId),
    InvalidPointReference(EntityPointRef),
    UnsupportedConstraintTarget(SketchConstraintId),
    SingularSystem,
    DidNotConverge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchSolveResult {
    pub status: SketchSolveStatus,
    pub iterations: usize,
    pub residual_norm: Scalar,
    pub dof: Option<usize>,
    pub error: Option<SketchSolveError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchEntity {
    pub id: SketchEntityId,
    pub construction: bool,
    pub name: Option<String>,
    pub geometry: SketchGeometry,
}

impl SketchEntity {
    pub fn bounds(&self) -> SketchBounds {
        self.geometry.bounds()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SketchGeometry {
    Point(SketchPoint),
    LineSegment(LineSegment),
    Circle(Circle),
    Arc(Arc),
}

impl SketchGeometry {
    pub fn bounds(&self) -> SketchBounds {
        match self {
            Self::Point(point) => SketchBounds::from_points([point.position]),
            Self::LineSegment(line) => SketchBounds::from_points([line.start, line.end]),
            Self::Circle(circle) => circle.bounds(),
            Self::Arc(arc) => arc.bounds(),
        }
    }

    pub fn point(&self, kind: EntityPointKind) -> Option<Point2> {
        match (self, kind) {
            (Self::Point(point), EntityPointKind::Position) => Some(point.position),
            (Self::LineSegment(line), EntityPointKind::Start) => Some(line.start),
            (Self::LineSegment(line), EntityPointKind::End) => Some(line.end),
            (Self::Circle(circle), EntityPointKind::Center) => Some(circle.center),
            (Self::Arc(arc), EntityPointKind::Center) => Some(arc.center),
            (Self::Arc(arc), EntityPointKind::Start) => Some(arc.start_point()),
            (Self::Arc(arc), EntityPointKind::End) => Some(arc.end_point()),
            _ => None,
        }
    }

    pub fn supports_point_kind(&self, kind: EntityPointKind) -> bool {
        self.point(kind).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SketchPoint {
    pub position: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LineSegment {
    pub start: Point2,
    pub end: Point2,
}

impl LineSegment {
    pub fn vector(&self) -> Vec2 {
        self.end - self.start
    }

    pub fn direction(&self) -> Vec2 {
        self.vector().normalize_or_zero()
    }

    pub fn length(&self) -> Scalar {
        self.start.distance(self.end)
    }

    pub fn closest_point(&self, point: Point2) -> Point2 {
        let ab = self.vector();
        let ab_len_sq = ab.length_squared();
        if ab_len_sq <= EPSILON {
            return self.start;
        }
        let t = ((point - self.start).dot(ab) / ab_len_sq).clamp(0.0, 1.0);
        self.start + ab * t
    }

    pub fn point_distance(&self, point: Point2) -> Scalar {
        point.distance(self.closest_point(point))
    }

    pub fn intersection(&self, other: &LineSegment) -> Option<Point2> {
        let p = self.start;
        let r = self.vector();
        let q = other.start;
        let s = other.vector();
        let rxs = cross_2d(r, s);
        if nearly_zero(rxs) {
            return None;
        }
        let q_p = q - p;
        let t = cross_2d(q_p, s) / rxs;
        let u = cross_2d(q_p, r) / rxs;
        if unit_interval().contains(&t) && unit_interval().contains(&u) {
            Some(p + r * t)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Circle {
    pub center: Point2,
    pub radius: Scalar,
}

impl Circle {
    pub fn bounds(&self) -> SketchBounds {
        SketchBounds {
            min: self.center - Vec2::splat(self.radius),
            max: self.center + Vec2::splat(self.radius),
        }
    }

    pub fn point_distance(&self, point: Point2) -> Scalar {
        (point.distance(self.center) - self.radius).abs()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Arc {
    pub center: Point2,
    pub radius: Scalar,
    pub start_angle: Scalar,
    pub end_angle: Scalar,
}

impl Arc {
    pub fn start_point(&self) -> Point2 {
        self.point_at_angle(self.start_angle)
    }

    pub fn end_point(&self) -> Point2 {
        self.point_at_angle(self.end_angle)
    }

    pub fn point_at_angle(&self, angle: Scalar) -> Point2 {
        self.center + Vec2::new(angle.cos(), angle.sin()) * self.radius
    }

    pub fn angle_span(&self) -> Scalar {
        let mut span = self.end_angle - self.start_angle;
        while span < 0.0 {
            span += std::f64::consts::TAU;
        }
        span
    }

    pub fn bounds(&self) -> SketchBounds {
        let mut points = vec![self.start_point(), self.end_point()];
        for angle in [0.0, std::f64::consts::FRAC_PI_2, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2] {
            if self.contains_angle(angle) {
                points.push(self.point_at_angle(angle));
            }
        }
        points.into_iter().collect()
    }

    pub fn contains_angle(&self, angle: Scalar) -> bool {
        let start = normalize_angle(self.start_angle);
        let end = normalize_angle(self.end_angle);
        let angle = normalize_angle(angle);
        if start <= end {
            angle >= start - EPSILON && angle <= end + EPSILON
        } else {
            angle >= start - EPSILON || angle <= end + EPSILON
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SketchBounds {
    pub min: Point2,
    pub max: Point2,
}

impl SketchBounds {
    pub fn from_points<const N: usize>(points: [Point2; N]) -> Self {
        points.into_iter().collect()
    }

    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
}

impl FromIterator<Point2> for SketchBounds {
    fn from_iter<I: IntoIterator<Item = Point2>>(points: I) -> Self {
        let mut iter = points.into_iter();
        let first = iter.next().unwrap_or(Point2::ZERO);
        let mut min = first;
        let mut max = first;
        for point in iter {
            min = min.min(point);
            max = max.max(point);
        }
        Self { min, max }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParameterTarget {
    PointX(SketchEntityId),
    PointY(SketchEntityId),
    LineStartX(SketchEntityId),
    LineStartY(SketchEntityId),
    LineEndX(SketchEntityId),
    LineEndY(SketchEntityId),
    CircleCenterX(SketchEntityId),
    CircleCenterY(SketchEntityId),
    CircleRadius(SketchEntityId),
    ArcCenterX(SketchEntityId),
    ArcCenterY(SketchEntityId),
    ArcRadius(SketchEntityId),
    ArcStartAngle(SketchEntityId),
    ArcEndAngle(SketchEntityId),
}

#[derive(Debug, Clone)]
struct SketchParameters {
    values: DVector<Scalar>,
    targets: Vec<ParameterTarget>,
}

impl SketchParameters {
    fn from_sketch(sketch: &Sketch) -> Result<Self, SketchSolveError> {
        let mut values = Vec::new();
        let mut targets = Vec::new();

        for entity in &sketch.entities {
            match entity.geometry {
                SketchGeometry::Point(point) => {
                    values.push(point.position.x);
                    targets.push(ParameterTarget::PointX(entity.id));
                    values.push(point.position.y);
                    targets.push(ParameterTarget::PointY(entity.id));
                }
                SketchGeometry::LineSegment(line) => {
                    values.push(line.start.x);
                    targets.push(ParameterTarget::LineStartX(entity.id));
                    values.push(line.start.y);
                    targets.push(ParameterTarget::LineStartY(entity.id));
                    values.push(line.end.x);
                    targets.push(ParameterTarget::LineEndX(entity.id));
                    values.push(line.end.y);
                    targets.push(ParameterTarget::LineEndY(entity.id));
                }
                SketchGeometry::Circle(circle) => {
                    values.push(circle.center.x);
                    targets.push(ParameterTarget::CircleCenterX(entity.id));
                    values.push(circle.center.y);
                    targets.push(ParameterTarget::CircleCenterY(entity.id));
                    values.push(circle.radius);
                    targets.push(ParameterTarget::CircleRadius(entity.id));
                }
                SketchGeometry::Arc(arc) => {
                    values.push(arc.center.x);
                    targets.push(ParameterTarget::ArcCenterX(entity.id));
                    values.push(arc.center.y);
                    targets.push(ParameterTarget::ArcCenterY(entity.id));
                    values.push(arc.radius);
                    targets.push(ParameterTarget::ArcRadius(entity.id));
                    values.push(arc.start_angle);
                    targets.push(ParameterTarget::ArcStartAngle(entity.id));
                    values.push(arc.end_angle);
                    targets.push(ParameterTarget::ArcEndAngle(entity.id));
                }
            }
        }

        Ok(Self {
            values: DVector::from_vec(values),
            targets,
        })
    }

    fn apply_to_sketch(&self, sketch: &mut Sketch) {
        for (index, target) in self.targets.iter().enumerate() {
            let value = self.values[index];
            if let Some(entity) = sketch.entity_mut(target.entity_id()) {
                match (&mut entity.geometry, target) {
                    (SketchGeometry::Point(point), ParameterTarget::PointX(_)) => point.position.x = value,
                    (SketchGeometry::Point(point), ParameterTarget::PointY(_)) => point.position.y = value,
                    (SketchGeometry::LineSegment(line), ParameterTarget::LineStartX(_)) => line.start.x = value,
                    (SketchGeometry::LineSegment(line), ParameterTarget::LineStartY(_)) => line.start.y = value,
                    (SketchGeometry::LineSegment(line), ParameterTarget::LineEndX(_)) => line.end.x = value,
                    (SketchGeometry::LineSegment(line), ParameterTarget::LineEndY(_)) => line.end.y = value,
                    (SketchGeometry::Circle(circle), ParameterTarget::CircleCenterX(_)) => circle.center.x = value,
                    (SketchGeometry::Circle(circle), ParameterTarget::CircleCenterY(_)) => circle.center.y = value,
                    (SketchGeometry::Circle(circle), ParameterTarget::CircleRadius(_)) => circle.radius = value.max(EPSILON),
                    (SketchGeometry::Arc(arc), ParameterTarget::ArcCenterX(_)) => arc.center.x = value,
                    (SketchGeometry::Arc(arc), ParameterTarget::ArcCenterY(_)) => arc.center.y = value,
                    (SketchGeometry::Arc(arc), ParameterTarget::ArcRadius(_)) => arc.radius = value.max(EPSILON),
                    (SketchGeometry::Arc(arc), ParameterTarget::ArcStartAngle(_)) => arc.start_angle = value,
                    (SketchGeometry::Arc(arc), ParameterTarget::ArcEndAngle(_)) => arc.end_angle = value,
                    _ => {}
                }
            }
        }
    }

    fn point_components(&self, point_ref: EntityPointRef) -> Option<(Scalar, Scalar)> {
        match point_ref.kind {
            EntityPointKind::Position => Some((
                self.value(ParameterTarget::PointX(point_ref.entity))?,
                self.value(ParameterTarget::PointY(point_ref.entity))?,
            )),
            EntityPointKind::Start => {
                if let (Some(x), Some(y)) = (
                    self.value(ParameterTarget::LineStartX(point_ref.entity)),
                    self.value(ParameterTarget::LineStartY(point_ref.entity)),
                ) {
                    Some((x, y))
                } else if let (Some(cx), Some(cy), Some(radius), Some(angle)) = (
                    self.value(ParameterTarget::ArcCenterX(point_ref.entity)),
                    self.value(ParameterTarget::ArcCenterY(point_ref.entity)),
                    self.value(ParameterTarget::ArcRadius(point_ref.entity)),
                    self.value(ParameterTarget::ArcStartAngle(point_ref.entity)),
                ) {
                    Some((cx + angle.cos() * radius, cy + angle.sin() * radius))
                } else {
                    None
                }
            }
            EntityPointKind::End => {
                if let (Some(x), Some(y)) = (
                    self.value(ParameterTarget::LineEndX(point_ref.entity)),
                    self.value(ParameterTarget::LineEndY(point_ref.entity)),
                ) {
                    Some((x, y))
                } else if let (Some(cx), Some(cy), Some(radius), Some(angle)) = (
                    self.value(ParameterTarget::ArcCenterX(point_ref.entity)),
                    self.value(ParameterTarget::ArcCenterY(point_ref.entity)),
                    self.value(ParameterTarget::ArcRadius(point_ref.entity)),
                    self.value(ParameterTarget::ArcEndAngle(point_ref.entity)),
                ) {
                    Some((cx + angle.cos() * radius, cy + angle.sin() * radius))
                } else {
                    None
                }
            }
            EntityPointKind::Center => {
                if let (Some(x), Some(y)) = (
                    self.value(ParameterTarget::CircleCenterX(point_ref.entity)),
                    self.value(ParameterTarget::CircleCenterY(point_ref.entity)),
                ) {
                    Some((x, y))
                } else if let (Some(x), Some(y)) = (
                    self.value(ParameterTarget::ArcCenterX(point_ref.entity)),
                    self.value(ParameterTarget::ArcCenterY(point_ref.entity)),
                ) {
                    Some((x, y))
                } else {
                    None
                }
            }
        }
    }

    fn value(&self, target: ParameterTarget) -> Option<Scalar> {
        let index = self.targets.iter().position(|candidate| *candidate == target)?;
        Some(self.values[index])
    }
}

impl ParameterTarget {
    fn entity_id(self) -> SketchEntityId {
        match self {
            Self::PointX(id)
            | Self::PointY(id)
            | Self::LineStartX(id)
            | Self::LineStartY(id)
            | Self::LineEndX(id)
            | Self::LineEndY(id)
            | Self::CircleCenterX(id)
            | Self::CircleCenterY(id)
            | Self::CircleRadius(id)
            | Self::ArcCenterX(id)
            | Self::ArcCenterY(id)
            | Self::ArcRadius(id)
            | Self::ArcStartAngle(id)
            | Self::ArcEndAngle(id) => id,
        }
    }
}

fn evaluate_residuals(sketch: &Sketch, values: &DVector<Scalar>) -> Result<DVector<Scalar>, SketchSolveError> {
    let parameters = SketchParameters {
        values: values.clone(),
        targets: SketchParameters::from_sketch(sketch)?.targets,
    };

    let mut residuals = Vec::new();
    for constraint in sketch.constraints.iter().filter(|constraint| constraint.enabled) {
        match &constraint.kind {
            SketchConstraintKind::Coincident { a, b } => {
                let (ax, ay) = parameters
                    .point_components(*a)
                    .ok_or(SketchSolveError::InvalidPointReference(*a))?;
                let (bx, by) = parameters
                    .point_components(*b)
                    .ok_or(SketchSolveError::InvalidPointReference(*b))?;
                residuals.push(ax - bx);
                residuals.push(ay - by);
            }
            SketchConstraintKind::Horizontal { line } => {
                let y1 = parameters
                    .value(ParameterTarget::LineStartY(*line))
                    .ok_or(SketchSolveError::UnsupportedConstraintTarget(constraint.id))?;
                let y2 = parameters
                    .value(ParameterTarget::LineEndY(*line))
                    .ok_or(SketchSolveError::UnsupportedConstraintTarget(constraint.id))?;
                residuals.push(y1 - y2);
            }
            SketchConstraintKind::Vertical { line } => {
                let x1 = parameters
                    .value(ParameterTarget::LineStartX(*line))
                    .ok_or(SketchSolveError::UnsupportedConstraintTarget(constraint.id))?;
                let x2 = parameters
                    .value(ParameterTarget::LineEndX(*line))
                    .ok_or(SketchSolveError::UnsupportedConstraintTarget(constraint.id))?;
                residuals.push(x1 - x2);
            }
            SketchConstraintKind::Distance { a, b, distance } => {
                let (ax, ay) = parameters
                    .point_components(*a)
                    .ok_or(SketchSolveError::InvalidPointReference(*a))?;
                let (bx, by) = parameters
                    .point_components(*b)
                    .ok_or(SketchSolveError::InvalidPointReference(*b))?;
                let dx = ax - bx;
                let dy = ay - by;
                residuals.push((dx * dx + dy * dy).sqrt() - *distance);
            }
        }
    }
    Ok(DVector::from_vec(residuals))
}

fn numerical_jacobian(
    sketch: &Sketch,
    values: &DVector<Scalar>,
    base_residual: &DVector<Scalar>,
) -> Result<DMatrix<Scalar>, SketchSolveError> {
    let rows = base_residual.len();
    let cols = values.len();
    let mut jacobian = DMatrix::<Scalar>::zeros(rows, cols);

    for column in 0..cols {
        let mut perturbed = values.clone();
        perturbed[column] += SOLVER_NUMERICAL_EPSILON;
        let residual = evaluate_residuals(sketch, &perturbed)?;
        let derivative = (&residual - base_residual) / SOLVER_NUMERICAL_EPSILON;
        jacobian.set_column(column, &derivative);
    }

    Ok(jacobian)
}

fn cross_2d(a: Vec2, b: Vec2) -> Scalar {
    a.x * b.y - a.y * b.x
}

fn unit_interval() -> RangeInclusive<Scalar> {
    0.0..=1.0
}

fn normalize_angle(mut angle: Scalar) -> Scalar {
    while angle < 0.0 {
        angle += std::f64::consts::TAU;
    }
    while angle >= std::f64::consts::TAU {
        angle -= std::f64::consts::TAU;
    }
    angle
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    #[test]
    fn line_length_and_direction() {
        let line = LineSegment {
            start: Point2::new(0.0, 0.0),
            end: Point2::new(3.0, 4.0),
        };
        assert_relative_eq!(line.length(), 5.0, epsilon = EPSILON);
        assert_relative_eq!(line.direction().x, 0.6, epsilon = EPSILON);
        assert_relative_eq!(line.direction().y, 0.8, epsilon = EPSILON);
    }

    #[test]
    fn circle_bounds_and_distance() {
        let circle = Circle {
            center: Point2::new(2.0, 3.0),
            radius: 5.0,
        };
        let bounds = circle.bounds();
        assert_relative_eq!(bounds.min.x, -3.0, epsilon = EPSILON);
        assert_relative_eq!(bounds.max.y, 8.0, epsilon = EPSILON);
        assert_relative_eq!(circle.point_distance(Point2::new(7.0, 3.0)), 0.0, epsilon = EPSILON);
    }

    #[test]
    fn arc_endpoints_and_bounds() {
        let arc = Arc {
            center: Point2::ZERO,
            radius: 2.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
        };
        let start = arc.start_point();
        let end = arc.end_point();
        let bounds = arc.bounds();
        assert_relative_eq!(start.x, 2.0, epsilon = EPSILON);
        assert_relative_eq!(start.y, 0.0, epsilon = EPSILON);
        assert_relative_eq!(end.x, 0.0, epsilon = EPSILON);
        assert_relative_eq!(end.y, 2.0, epsilon = EPSILON);
        assert_relative_eq!(bounds.max.x, 2.0, epsilon = EPSILON);
        assert_relative_eq!(bounds.max.y, 2.0, epsilon = EPSILON);
    }

    #[test]
    fn line_intersection_finds_crossing_point() {
        let a = LineSegment {
            start: Point2::new(0.0, 0.0),
            end: Point2::new(2.0, 2.0),
        };
        let b = LineSegment {
            start: Point2::new(0.0, 2.0),
            end: Point2::new(2.0, 0.0),
        };
        let hit = a.intersection(&b).expect("expected intersection");
        assert_relative_eq!(hit.x, 1.0, epsilon = EPSILON);
        assert_relative_eq!(hit.y, 1.0, epsilon = EPSILON);
    }

    #[test]
    fn sketch_add_lookup_remove_entities() {
        let mut sketch = Sketch::world_xy();
        let id = sketch.add_entity(
            SketchGeometry::Point(SketchPoint {
                position: Point2::new(1.0, 2.0),
            }),
            false,
            Some("p1".into()),
        );
        assert!(sketch.entity(id).is_some());
        let removed = sketch.remove_entity(id).expect("entity should exist");
        assert_eq!(removed.id, id);
        assert!(sketch.entity(id).is_none());
    }

    #[test]
    fn sketch_plane_converts_local_and_world_points() {
        let sketch = Sketch::world_xy();
        let world = sketch.local_point_to_world(Point2::new(3.0, -4.0));
        let local = sketch.world_point_to_local(Point3::new(3.0, -4.0, 7.0));
        assert_relative_eq!(world.x, 3.0, epsilon = EPSILON);
        assert_relative_eq!(world.y, -4.0, epsilon = EPSILON);
        assert_relative_eq!(world.z, 0.0, epsilon = EPSILON);
        assert_relative_eq!(local.x, 3.0, epsilon = EPSILON);
        assert_relative_eq!(local.y, -4.0, epsilon = EPSILON);
    }

    #[test]
    fn sketch_add_lookup_remove_constraints() {
        let mut sketch = Sketch::world_xy();
        let point_id = sketch.add_entity(
            SketchGeometry::Point(SketchPoint {
                position: Point2::new(1.0, 2.0),
            }),
            false,
            Some("p1".into()),
        );
        let line_id = sketch.add_entity(
            SketchGeometry::LineSegment(LineSegment {
                start: Point2::new(0.0, 0.0),
                end: Point2::new(2.0, 0.0),
            }),
            false,
            Some("l1".into()),
        );
        let constraint_id = sketch.add_constraint(
            SketchConstraintKind::Distance {
                a: EntityPointRef {
                    entity: point_id,
                    kind: EntityPointKind::Position,
                },
                b: EntityPointRef {
                    entity: line_id,
                    kind: EntityPointKind::Start,
                },
                distance: 5.0,
            },
            Some("d1".into()),
        );
        let constraint = sketch.constraint(constraint_id).expect("constraint should exist");
        match &constraint.kind {
            SketchConstraintKind::Distance { distance, .. } => {
                assert_relative_eq!(*distance, 5.0, epsilon = EPSILON);
            }
            _ => panic!("expected distance constraint"),
        }
        let removed = sketch.remove_constraint(constraint_id).expect("constraint should exist");
        assert_eq!(removed.id, constraint_id);
        assert!(sketch.constraint(constraint_id).is_none());
    }

    #[test]
    fn geometry_point_refs_resolve_expected_points() {
        let mut sketch = Sketch::world_xy();
        let line_id = sketch.add_entity(
            SketchGeometry::LineSegment(LineSegment {
                start: Point2::new(1.0, 2.0),
                end: Point2::new(3.0, 4.0),
            }),
            false,
            Some("line".into()),
        );
        let arc_id = sketch.add_entity(
            SketchGeometry::Arc(Arc {
                center: Point2::new(10.0, 20.0),
                radius: 5.0,
                start_angle: 0.0,
                end_angle: std::f64::consts::FRAC_PI_2,
            }),
            false,
            Some("arc".into()),
        );

        let line_start = sketch.resolve_point_ref(EntityPointRef {
            entity: line_id,
            kind: EntityPointKind::Start,
        }).expect("line start should resolve");
        let arc_end = sketch.resolve_point_ref(EntityPointRef {
            entity: arc_id,
            kind: EntityPointKind::End,
        }).expect("arc end should resolve");

        assert_relative_eq!(line_start.x, 1.0, epsilon = EPSILON);
        assert_relative_eq!(line_start.y, 2.0, epsilon = EPSILON);
        assert_relative_eq!(arc_end.x, 10.0, epsilon = EPSILON);
        assert_relative_eq!(arc_end.y, 25.0, epsilon = EPSILON);
    }

    #[test]
    fn invalid_point_refs_are_rejected() {
        let mut sketch = Sketch::world_xy();
        let circle_id = sketch.add_entity(
            SketchGeometry::Circle(Circle {
                center: Point2::new(0.0, 0.0),
                radius: 2.0,
            }),
            false,
            None,
        );

        let invalid = sketch.resolve_point_ref(EntityPointRef {
            entity: circle_id,
            kind: EntityPointKind::Start,
        });

        assert!(invalid.is_none());
    }

    #[test]
    fn coincident_residual_is_zero_for_matching_points() {
        let mut sketch = Sketch::world_xy();
        let a = sketch.add_entity(
            SketchGeometry::Point(SketchPoint { position: Point2::new(1.0, 2.0) }),
            false,
            None,
        );
        let b = sketch.add_entity(
            SketchGeometry::Point(SketchPoint { position: Point2::new(1.0, 2.0) }),
            false,
            None,
        );
        sketch.add_constraint(
            SketchConstraintKind::Coincident {
                a: EntityPointRef { entity: a, kind: EntityPointKind::Position },
                b: EntityPointRef { entity: b, kind: EntityPointKind::Position },
            },
            None,
        );

        let params = SketchParameters::from_sketch(&sketch).unwrap();
        let residual = evaluate_residuals(&sketch, &params.values).unwrap();
        assert_relative_eq!(residual.norm(), 0.0, epsilon = EPSILON);
    }

    #[test]
    fn horizontal_and_vertical_residuals_detect_unsatisfied_lines() {
        let mut sketch = Sketch::world_xy();
        let horizontal = sketch.add_entity(
            SketchGeometry::LineSegment(LineSegment {
                start: Point2::new(0.0, 0.0),
                end: Point2::new(1.0, 2.0),
            }),
            false,
            None,
        );
        let vertical = sketch.add_entity(
            SketchGeometry::LineSegment(LineSegment {
                start: Point2::new(0.0, 0.0),
                end: Point2::new(2.0, 1.0),
            }),
            false,
            None,
        );
        sketch.add_constraint(SketchConstraintKind::Horizontal { line: horizontal }, None);
        sketch.add_constraint(SketchConstraintKind::Vertical { line: vertical }, None);

        let params = SketchParameters::from_sketch(&sketch).unwrap();
        let residual = evaluate_residuals(&sketch, &params.values).unwrap();
        assert_relative_eq!(residual[0], -2.0, epsilon = EPSILON);
        assert_relative_eq!(residual[1], -2.0, epsilon = EPSILON);
    }

    #[test]
    fn distance_constraint_moves_geometry_when_solved() {
        let mut sketch = Sketch::world_xy();
        let a = sketch.add_entity(
            SketchGeometry::Point(SketchPoint { position: Point2::new(0.0, 0.0) }),
            false,
            None,
        );
        let b = sketch.add_entity(
            SketchGeometry::Point(SketchPoint { position: Point2::new(1.0, 0.0) }),
            false,
            None,
        );
        sketch.add_constraint(
            SketchConstraintKind::Distance {
                a: EntityPointRef { entity: a, kind: EntityPointKind::Position },
                b: EntityPointRef { entity: b, kind: EntityPointKind::Position },
                distance: 5.0,
            },
            None,
        );

        let result = sketch.solve();
        assert_eq!(result.status, SketchSolveStatus::Converged);
        let pa = sketch.resolve_point_ref(EntityPointRef { entity: a, kind: EntityPointKind::Position }).unwrap();
        let pb = sketch.resolve_point_ref(EntityPointRef { entity: b, kind: EntityPointKind::Position }).unwrap();
        assert_relative_eq!(pa.distance(pb), 5.0, epsilon = 1.0e-5);
    }

    #[test]
    fn contradictory_constraints_fail_cleanly() {
        let mut sketch = Sketch::world_xy();
        let line = sketch.add_entity(
            SketchGeometry::LineSegment(LineSegment {
                start: Point2::new(0.0, 0.0),
                end: Point2::new(1.0, 1.0),
            }),
            false,
            None,
        );
        sketch.add_constraint(SketchConstraintKind::Horizontal { line }, None);
        sketch.add_constraint(SketchConstraintKind::Vertical { line }, None);
        let result = sketch.solve();
        assert!(matches!(result.status, SketchSolveStatus::Converged | SketchSolveStatus::MaxIterationsReached));
    }

    #[test]
    fn constraint_visuals_are_generated_for_all_constraints() {
        let mut sketch = Sketch::world_xy();
        let a = sketch.add_entity(
            SketchGeometry::Point(SketchPoint { position: Point2::new(0.0, 0.0) }),
            false,
            None,
        );
        let b = sketch.add_entity(
            SketchGeometry::Point(SketchPoint { position: Point2::new(5.0, 0.0) }),
            false,
            None,
        );
        sketch.add_constraint(
            SketchConstraintKind::Distance {
                a: EntityPointRef { entity: a, kind: EntityPointKind::Position },
                b: EntityPointRef { entity: b, kind: EntityPointKind::Position },
                distance: 10.0,
            },
            None,
        );
        
        let result = sketch.solve();
        let visuals = sketch.get_constraint_visuals(&result);
        
        assert_eq!(visuals.len(), 1);
        assert_eq!(visuals[0].state, ConstraintState::Satisfied);
    }

    #[test]
    fn extract_wire_from_rectangle() {
        let mut sketch = Sketch::world_xy();
        
        // Create a rectangle from (0,0) to (10,5)
        let corners = sketch.add_rectangle(Point2::new(0.0, 0.0), Point2::new(10.0, 5.0));
        
        // Extract the closed wire
        let wire_result = sketch.extract_closed_wire();
        assert!(wire_result.is_ok(), "Wire extraction should succeed");
        
        let wire = wire_result.unwrap();
        assert_eq!(wire.edges.len(), 4, "Rectangle should have exactly 4 edges");
        
        // Verify all 4 rectangle edges are in the result
        let edge_set: std::collections::HashSet<SketchEntityId> = wire.edges.iter().cloned().collect();
        assert!(edge_set.contains(&corners[0]), "Wire should contain bottom edge");
        assert!(edge_set.contains(&corners[1]), "Wire should contain right edge");
        assert!(edge_set.contains(&corners[2]), "Wire should contain top edge");
        assert!(edge_set.contains(&corners[3]), "Wire should contain left edge");
        
        // Verify edges are properly connected (each edge's end connects to next edge's start)
        let verify_edge_connectivity = |id1: SketchEntityId, id2: SketchEntityId| -> bool {
            let line1 = sketch.entity(id1).unwrap();
            let line2 = sketch.entity(id2).unwrap();
            if let (SketchGeometry::LineSegment(l1), SketchGeometry::LineSegment(l2)) = (&line1.geometry, &line2.geometry) {
                let match_end_to_start = (l1.end - l2.start).length() < EPSILON;
                let match_end_to_end = (l1.end - l2.end).length() < EPSILON;
                let match_start_to_start = (l1.start - l2.start).length() < EPSILON;
                let match_start_to_end = (l1.start - l2.end).length() < EPSILON;
                // l1 and l2 should share an endpoint
                match_end_to_start || match_end_to_end || match_start_to_start || match_start_to_end
            } else {
                false
            }
        };
        
        // Check that consecutive edges are connected
        for i in 0..wire.edges.len() {
            let next_i = (i + 1) % wire.edges.len();
            assert!(
                verify_edge_connectivity(wire.edges[i], wire.edges[next_i]),
                "Edges {} and {} should be connected",
                i,
                next_i
            );
        }
    }
}
