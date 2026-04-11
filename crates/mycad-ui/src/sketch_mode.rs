//! Sketch mode: 2D drawing, constraint display, snap indicators.

use mycad_kernel::math::{Point2, Scalar};
use mycad_kernel::sketch::{EntityPointKind, EntityPointRef, Sketch, SketchEntityId};

/// Item that can be selected in sketch mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectableItem {
    Entity(SketchEntityId),
    Point(EntityPointRef),
}

/// State for the select tool (multi-selection support)
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    pub selected_items: Vec<SelectableItem>,
    pub hover_item: Option<SelectableItem>,
}

impl SelectionState {
    pub fn clear(&mut self) {
        self.selected_items.clear();
        self.hover_item = None;
    }

    pub fn toggle_selection(&mut self, item: SelectableItem) {
        if let Some(pos) = self.selected_items.iter().position(|&i| i == item) {
            self.selected_items.remove(pos);
        } else {
            self.selected_items.push(item);
        }
    }

    pub fn is_selected(&self, item: SelectableItem) -> bool {
        self.selected_items.contains(&item)
    }
}

/// Find the closest selectable item to a point within a threshold
pub fn pick_sketch_item(
    sketch: &Sketch,
    point: Point2,
    threshold: Scalar,
) -> Option<SelectableItem> {
    let mut best_dist = threshold;
    let mut best_item = None;

    // Check all entity points first (higher priority)
    for entity in sketch.iter() {
        // Check entity-specific points
        match &entity.geometry {
            mycad_kernel::sketch::SketchGeometry::Point(pt) => {
                let d = pt.position.distance(point);
                if d < best_dist {
                    best_dist = d;
                    best_item = Some(SelectableItem::Point(EntityPointRef {
                        entity: entity.id,
                        kind: EntityPointKind::Position,
                    }));
                }
            }
            mycad_kernel::sketch::SketchGeometry::LineSegment(line) => {
                // Check start point
                let d_start = line.start.distance(point);
                if d_start < best_dist {
                    best_dist = d_start;
                    best_item = Some(SelectableItem::Point(EntityPointRef {
                        entity: entity.id,
                        kind: EntityPointKind::Start,
                    }));
                }
                // Check end point
                let d_end = line.end.distance(point);
                if d_end < best_dist {
                    best_dist = d_end;
                    best_item = Some(SelectableItem::Point(EntityPointRef {
                        entity: entity.id,
                        kind: EntityPointKind::End,
                    }));
                }
                // Check line itself (lower priority than points)
                let d_line = line.point_distance(point);
                if d_line < best_dist * 1.5 && best_item.is_none() {
                    // Only select line if no point is close
                    best_item = Some(SelectableItem::Entity(entity.id));
                }
            }
            mycad_kernel::sketch::SketchGeometry::Circle(circle) => {
                // Check center point
                let d_center = circle.center.distance(point);
                if d_center < best_dist {
                    best_dist = d_center;
                    best_item = Some(SelectableItem::Point(EntityPointRef {
                        entity: entity.id,
                        kind: EntityPointKind::Center,
                    }));
                }
            }
            mycad_kernel::sketch::SketchGeometry::Arc(arc) => {
                // Check center point
                let d_center = arc.center.distance(point);
                if d_center < best_dist {
                    best_dist = d_center;
                    best_item = Some(SelectableItem::Point(EntityPointRef {
                        entity: entity.id,
                        kind: EntityPointKind::Center,
                    }));
                }
                // Check endpoints
                let d_start = arc.start_point().distance(point);
                if d_start < best_dist {
                    best_dist = d_start;
                    best_item = Some(SelectableItem::Point(EntityPointRef {
                        entity: entity.id,
                        kind: EntityPointKind::Start,
                    }));
                }
                let d_end = arc.end_point().distance(point);
                if d_end < best_dist {
                    best_dist = d_end;
                    best_item = Some(SelectableItem::Point(EntityPointRef {
                        entity: entity.id,
                        kind: EntityPointKind::End,
                    }));
                }
            }
        }
    }

    best_item
}

/// Compute colors for sketch entities based on constraint state
pub fn entity_color(
    _entity_id: SketchEntityId,
    is_selected: bool,
    is_hover: bool,
) -> [f32; 4] {
    if is_selected {
        [1.0, 1.0, 0.3, 1.0] // Yellow for selected
    } else if is_hover {
        [0.5, 1.0, 0.5, 1.0] // Light green for hover
    } else {
        [0.2, 0.8, 1.0, 1.0] // Default cyan
    }
}

/// Color for constraint visualization based on state
pub fn constraint_color(state: mycad_kernel::sketch::ConstraintState) -> [f32; 4] {
    use mycad_kernel::sketch::ConstraintState;
    match state {
        ConstraintState::Satisfied => [0.3, 1.0, 0.3, 1.0],      // Green
        ConstraintState::Conflicting => [1.0, 0.3, 0.3, 1.0],   // Red
        ConstraintState::OverConstrained => [1.0, 0.6, 0.2, 1.0], // Orange
        ConstraintState::Normal => [0.8, 0.8, 0.8, 1.0],        // Gray
    }
}
