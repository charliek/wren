//! File-drag arithmetic and state transitions. There are no filesystem effects here.
//! Visibility is geometry, not existence: spring-opening may remove a source from view.

use std::time::Duration;

use iced::{Point, Rectangle};
use wren_core::model::{GroupId, ItemRef, NoteId, Parent, SectionId, Workspace};
use wren_core::ops::Op;

pub const THRESHOLD: f32 = 4.0;
pub const SPRING_DELAY: Duration = Duration::from_millis(600);
pub const DOUBLE_CLICK_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKind {
    Section(SectionId),
    Group(GroupId),
    Note(NoteId),
}

impl RowKind {
    pub fn item(self) -> ItemRef {
        match self {
            Self::Section(id) => ItemRef::Section(id),
            Self::Group(id) => ItemRef::Group(id),
            Self::Note(id) => ItemRef::Note(id),
        }
    }
}

/// A complete model snapshot in manual sibling order, including filtered/hidden rows.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    sections: Vec<SectionId>,
    groups: Vec<(GroupId, SectionId)>,
    notes: Vec<(NoteId, GroupId)>,
    labels: Vec<(RowKind, String)>,
}

impl Catalog {
    pub fn from_workspace(workspace: &Workspace) -> Self {
        let mut labels: Vec<_> = workspace
            .sections
            .iter()
            .map(|section| (RowKind::Section(section.id), section.name.clone()))
            .collect();
        labels.extend(
            workspace
                .groups
                .iter()
                .map(|group| (RowKind::Group(group.id), group.name.clone())),
        );
        labels.extend(
            workspace
                .notes
                .iter()
                .map(|note| (RowKind::Note(note.id), note.title.clone())),
        );
        Self {
            sections: workspace
                .sections
                .iter()
                .map(|section| section.id)
                .collect(),
            groups: workspace
                .groups
                .iter()
                .map(|group| (group.id, group.section))
                .collect(),
            notes: workspace
                .notes
                .iter()
                .map(|note| (note.id, note.group))
                .collect(),
            labels,
        }
    }

    pub fn contains(&self, row: RowKind) -> bool {
        match row {
            RowKind::Section(id) => self.sections.contains(&id),
            RowKind::Group(id) => self.groups.iter().any(|&(group, _)| group == id),
            RowKind::Note(id) => self.notes.iter().any(|&(note, _)| note == id),
        }
    }

    pub fn parent(&self, row: RowKind) -> Option<Parent> {
        match row {
            RowKind::Group(id) => self
                .groups
                .iter()
                .find_map(|&(group, section)| (group == id).then_some(Parent::Section(section))),
            RowKind::Note(id) => self
                .notes
                .iter()
                .find_map(|&(note, group)| (note == id).then_some(Parent::Group(group))),
            _ => None,
        }
    }

    pub fn label(&self, row: RowKind) -> &str {
        self.labels
            .iter()
            .find_map(|(kind, label)| (*kind == row).then_some(label.as_str()))
            .unwrap_or("Item")
    }

    pub fn destination_label(&self, row: RowKind) -> String {
        match row {
            RowKind::Group(group) => match self.parent(RowKind::Group(group)) {
                Some(Parent::Section(section)) => format!(
                    "{} › {}",
                    self.label(RowKind::Section(section)),
                    self.label(row)
                ),
                _ => self.label(row).to_owned(),
            },
            _ => self.label(row).to_owned(),
        }
    }

    fn siblings(&self, row: RowKind) -> Vec<ItemRef> {
        match row {
            RowKind::Section(_) => self
                .sections
                .iter()
                .copied()
                .map(ItemRef::Section)
                .collect(),
            RowKind::Group(_) => {
                let parent = self.parent(row);
                self.groups
                    .iter()
                    .filter(|(_, section)| Some(Parent::Section(*section)) == parent)
                    .map(|(group, _)| ItemRef::Group(*group))
                    .collect()
            }
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowGeometry {
    pub row: RowKind,
    /// Full row bounds in root/window coordinates; centers must not move when clipped.
    pub bounds: Rectangle,
    pub visible: Rectangle,
}

impl RowGeometry {
    pub fn clipped(row: RowKind, bounds: Rectangle, viewport: Rectangle) -> Option<Self> {
        let visible = bounds.intersection(&viewport)?;
        (visible.width > 0.0 && visible.height > 0.0).then_some(Self {
            row,
            bounds,
            visible,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub root: Rectangle,
    pub rows: Vec<RowGeometry>,
}

impl Geometry {
    pub fn new(root: Rectangle) -> Self {
        Self { root, rows: vec![] }
    }

    pub fn hit(&self, position: Point) -> Option<&RowGeometry> {
        if !self.root.contains(position) {
            return None;
        }
        self.rows
            .iter()
            .rev()
            .find(|row| row.visible.contains(position))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    MoveNote {
        note: NoteId,
        to: GroupId,
    },
    MoveGroup {
        group: GroupId,
        to: SectionId,
    },
    Reorder {
        target: ItemRef,
        before: Option<ItemRef>,
    },
}

impl DropAction {
    pub fn op(self) -> Op {
        match self {
            Self::MoveNote { note, to } => Op::MoveNote { note, to },
            Self::MoveGroup { group, to } => Op::MoveGroup { group, to },
            Self::Reorder { target, before } => Op::Reorder { target, before },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Insertion {
    pub edge: Edge,
    pub line: Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropTarget {
    pub row: RowKind,
    pub action: DropAction,
    pub insertion: Option<Insertion>,
    pub bounds: Rectangle,
}

/// Returns only meaningful allowed operations. Same-location moves and identity reorders are no-ops.
pub fn target(
    source: RowKind,
    hit: &RowGeometry,
    position: Point,
    catalog: &Catalog,
) -> Option<DropTarget> {
    if !catalog.contains(source) || !catalog.contains(hit.row) || source == hit.row {
        return None;
    }
    let mut insertion = None;
    let action = match (source, hit.row) {
        (RowKind::Note(note), RowKind::Group(group)) => DropAction::MoveNote { note, to: group },
        (RowKind::Group(group), RowKind::Section(to))
            if catalog.parent(source) != Some(Parent::Section(to)) =>
        {
            DropAction::MoveGroup { group, to }
        }
        (RowKind::Section(_), RowKind::Section(_)) | (RowKind::Group(_), RowKind::Group(_))
            if catalog.parent(source) == catalog.parent(hit.row) =>
        {
            let target = source.item();
            let over = hit.row.item();
            let edge = if position.y < hit.bounds.center_y() {
                Edge::Before
            } else {
                Edge::After
            };
            let siblings = catalog.siblings(source);
            let remaining: Vec<_> = siblings
                .iter()
                .copied()
                .filter(|item| *item != target)
                .collect();
            let at =
                remaining.iter().position(|item| *item == over)? + usize::from(edge == Edge::After);
            let before = remaining.get(at).copied();
            let original = siblings.iter().position(|item| *item == target)?;
            let mut reordered = remaining;
            reordered.insert(at, target);
            if reordered == siblings || original == at {
                return None;
            }
            let y = match edge {
                Edge::Before => hit.bounds.y,
                Edge::After => hit.bounds.y + hit.bounds.height - 2.0,
            };
            let line = Rectangle {
                x: hit.bounds.x,
                y,
                width: hit.bounds.width,
                height: 2.0,
            }
            .intersection(&hit.visible)?;
            if line.width <= 0.0 || line.height <= 0.0 {
                return None;
            }
            insertion = Some(Insertion { edge, line });
            DropAction::Reorder { target, before }
        }
        _ => return None,
    };
    if matches!(action, DropAction::MoveNote { to, .. } if Some(Parent::Group(to)) == catalog.parent(source))
    {
        return None;
    }
    Some(DropTarget {
        row: hit.row,
        action,
        insertion,
        bounds: hit.visible,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpringPreview {
    pub section: SectionId,
    pub opened: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub generation: u64,
    pub source: RowKind,
    pub position: Point,
    pub target: Option<DropTarget>,
    pub spring: Option<SpringPreview>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    Escape,
    Blur,
    Disabled,
    StaleGeneration,
    SourceRemoved,
    SourceChanged,
    TargetRemoved,
    TargetChanged,
    InvalidTarget,
    ReleasedElsewhere,
}

#[derive(Debug, Clone)]
pub enum DragEvent {
    Preview(Preview),
    /// Browser-only selection/expansion preview. Never dispatch OpenNote or Flush for this.
    Spring {
        generation: u64,
        section: SectionId,
    },
    Commit {
        generation: u64,
        op: Op,
    },
    Activate {
        generation: u64,
        row: RowKind,
    },
    /// A double-click: select the row and move one column right.
    Open {
        generation: u64,
        row: RowKind,
    },
    Context {
        generation: u64,
        row: RowKind,
    },
    Cancelled {
        generation: u64,
        reason: CancelReason,
    },
}

#[derive(Debug, Clone, Copy)]
struct Hover {
    section: SectionId,
    since: Duration,
    opened: bool,
}

#[derive(Debug, Clone, Copy)]
struct TargetReferences {
    destination: (RowKind, Option<Parent>),
    before: Option<(RowKind, Option<Parent>)>,
}

impl TargetReferences {
    fn new(target: DropTarget, catalog: &Catalog) -> Self {
        let before = match target.action {
            DropAction::Reorder {
                before: Some(item), ..
            } => Some(match item {
                ItemRef::Section(id) => RowKind::Section(id),
                ItemRef::Group(id) => RowKind::Group(id),
                ItemRef::Note(id) => RowKind::Note(id),
            }),
            _ => None,
        };
        Self {
            destination: (target.row, catalog.parent(target.row)),
            before: before.map(|row| (row, catalog.parent(row))),
        }
    }

    fn invalid(self, catalog: &Catalog) -> Option<CancelReason> {
        std::iter::once(self.destination)
            .chain(self.before)
            .find_map(|(row, parent)| {
                if !catalog.contains(row) {
                    Some(CancelReason::TargetRemoved)
                } else if catalog.parent(row) != parent {
                    Some(CancelReason::TargetChanged)
                } else {
                    None
                }
            })
    }
}

#[derive(Debug, Clone)]
struct Gesture {
    generation: u64,
    source: RowKind,
    parent: Option<Parent>,
    origin: Point,
    position: Point,
    dragging: bool,
    hover: Option<Hover>,
    target_refs: Option<TargetReferences>,
}

#[derive(Debug, Clone, Copy)]
struct ClickRecord {
    generation: u64,
    row: RowKind,
    parent: Option<Parent>,
    position: Point,
    at: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct Drag {
    gesture: Option<Gesture>,
    preview: Option<Preview>,
    release_pending: bool,
    last_click: Option<ClickRecord>,
}

impl Drag {
    pub fn is_pressed(&self) -> bool {
        self.release_pending
    }
    /// A fresh press cannot belong to a cancelled gesture's missing release.
    pub(crate) fn retire_cancelled_release(&mut self) {
        if self.gesture.is_none() {
            self.release_pending = false;
        }
    }
    pub fn is_active(&self) -> bool {
        self.gesture.is_some()
    }
    pub fn is_dragging(&self) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|gesture| gesture.dragging)
    }
    pub fn preview(&self) -> Option<&Preview> {
        self.preview.as_ref()
    }

    pub fn press(
        &mut self,
        row: RowKind,
        position: Point,
        generation: u64,
        catalog: &Catalog,
    ) -> bool {
        if self.gesture.is_some() || !catalog.contains(row) {
            return false;
        }
        self.release_pending = true;
        self.preview = None;
        self.gesture = Some(Gesture {
            generation,
            source: row,
            parent: catalog.parent(row),
            origin: position,
            position,
            dragging: false,
            hover: None,
            target_refs: None,
        });
        true
    }

    pub fn forget_click(&mut self) {
        self.last_click = None;
    }

    fn retain_click(&mut self, generation: u64, catalog: &Catalog, geometry: &Geometry) {
        if self.last_click.is_some_and(|click| {
            click.generation != generation
                || !catalog.contains(click.row)
                || catalog.parent(click.row) != click.parent
                || !geometry.rows.iter().any(|row| row.row == click.row)
        }) {
            self.last_click = None;
        }
    }

    pub fn cancel(&mut self, reason: CancelReason) -> Option<DragEvent> {
        self.forget_click();
        let gesture = self.gesture.take()?;
        self.preview = None;
        Some(DragEvent::Cancelled {
            generation: gesture.generation,
            reason,
        })
    }

    pub fn validate(&mut self, generation: u64, catalog: &Catalog) -> Option<DragEvent> {
        let gesture = self.gesture.as_ref()?;
        let reason = if gesture.generation != generation {
            CancelReason::StaleGeneration
        } else if !catalog.contains(gesture.source) {
            CancelReason::SourceRemoved
        } else if catalog.parent(gesture.source) != gesture.parent {
            CancelReason::SourceChanged
        } else {
            gesture.target_refs.and_then(|refs| refs.invalid(catalog))?
        };
        self.cancel(reason)
    }

    pub fn next_deadline(&self) -> Option<Duration> {
        self.gesture
            .as_ref()?
            .hover
            .filter(|hover| !hover.opened)
            .map(|hover| hover.since.saturating_add(SPRING_DELAY))
    }

    /// Re-run on every motion, scroll, filter/layout reflow, and scheduled spring redraw.
    pub fn motion(
        &mut self,
        position: Point,
        now: Duration,
        generation: u64,
        catalog: &Catalog,
        geometry: &Geometry,
    ) -> Vec<DragEvent> {
        if let Some(event) = self.validate(generation, catalog) {
            return vec![event];
        }
        let Some(gesture) = &mut self.gesture else {
            return vec![];
        };
        gesture.position = position;
        if !gesture.dragging {
            if gesture.origin.distance(position) < THRESHOLD {
                return vec![];
            }
            gesture.dragging = true;
            self.last_click = None;
        }
        let hit = geometry.hit(position);
        let hovered_section = match (gesture.source, hit.map(|row| row.row)) {
            (RowKind::Note(_) | RowKind::Group(_), Some(RowKind::Section(section))) => {
                Some(section)
            }
            _ => None,
        };
        if gesture.hover.map(|hover| hover.section) != hovered_section {
            gesture.hover = hovered_section.map(|section| Hover {
                section,
                since: now,
                opened: false,
            });
        }
        let mut events = vec![];
        if let Some(hover) = &mut gesture.hover {
            if !hover.opened && now.saturating_sub(hover.since) >= SPRING_DELAY {
                hover.opened = true;
                events.push(DragEvent::Spring {
                    generation: gesture.generation,
                    section: hover.section,
                });
            }
        }
        let preview = Preview {
            generation: gesture.generation,
            source: gesture.source,
            position,
            target: hit.and_then(|hit| target(gesture.source, hit, position, catalog)),
            spring: gesture.hover.map(|hover| SpringPreview {
                section: hover.section,
                opened: hover.opened,
            }),
        };
        gesture.target_refs = preview
            .target
            .map(|target| TargetReferences::new(target, catalog));
        if self.preview.as_ref() != Some(&preview) {
            self.preview = Some(preview.clone());
            events.push(DragEvent::Preview(preview));
        }
        events
    }

    pub fn refresh(
        &mut self,
        now: Duration,
        generation: u64,
        catalog: &Catalog,
        geometry: &Geometry,
    ) -> Vec<DragEvent> {
        self.retain_click(generation, catalog, geometry);
        let Some(position) = self.gesture.as_ref().map(|gesture| gesture.position) else {
            return vec![];
        };
        self.motion(position, now, generation, catalog, geometry)
    }

    /// A cursor that left the root window cannot retain a valid target or spring timer.
    pub fn pointer_left(
        &mut self,
        now: Duration,
        generation: u64,
        catalog: &Catalog,
        geometry: &Geometry,
    ) -> Vec<DragEvent> {
        self.forget_click();
        let outside = Point::new(geometry.root.x - 1000.0, geometry.root.y - 1000.0);
        if !self.is_dragging() {
            if let Some(gesture) = &mut self.gesture {
                gesture.hover = None;
            }
            return vec![];
        }
        self.motion(outside, now, generation, catalog, geometry)
    }

    /// One physical release consumes the gesture, even after cancel or an invalid drop.
    pub fn release(
        &mut self,
        position: Option<Point>,
        now: Duration,
        generation: u64,
        catalog: &Catalog,
        geometry: &Geometry,
    ) -> Vec<DragEvent> {
        if !std::mem::take(&mut self.release_pending) {
            return vec![];
        }
        let mut events = if let Some(position) = position {
            self.motion(position, now, generation, catalog, geometry)
        } else {
            self.validate(generation, catalog).into_iter().collect()
        };
        let Some(gesture) = self.gesture.take() else {
            return events;
        };
        let preview = self.preview.take();
        self.retain_click(generation, catalog, geometry);
        let previous_click = self.last_click.take();
        let event = if gesture.dragging {
            match position.and_then(|_| preview.and_then(|preview| preview.target)) {
                Some(target) => DragEvent::Commit {
                    generation: gesture.generation,
                    op: target.action.op(),
                },
                None => DragEvent::Cancelled {
                    generation: gesture.generation,
                    reason: CancelReason::InvalidTarget,
                },
            }
        } else if position
            .and_then(|position| geometry.hit(position))
            .is_some_and(|hit| hit.row == gesture.source)
        {
            let position = position.expect("row hit has a position");
            if previous_click.is_some_and(|click| {
                click.generation == gesture.generation
                    && click.row == gesture.source
                    && click.parent == gesture.parent
                    && now.saturating_sub(click.at) <= DOUBLE_CLICK_DELAY
                    && click.position.distance(gesture.origin) < THRESHOLD
                    && click.position.distance(position) < THRESHOLD
            }) {
                DragEvent::Open {
                    generation: gesture.generation,
                    row: gesture.source,
                }
            } else {
                self.last_click = Some(ClickRecord {
                    generation: gesture.generation,
                    row: gesture.source,
                    parent: gesture.parent,
                    position,
                    at: now,
                });
                DragEvent::Activate {
                    generation: gesture.generation,
                    row: gesture.source,
                }
            }
        } else {
            DragEvent::Cancelled {
                generation: gesture.generation,
                reason: CancelReason::ReleasedElsewhere,
            }
        };
        events.push(event);
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Size;

    fn catalog() -> Catalog {
        Catalog {
            sections: vec![SectionId(1), SectionId(2), SectionId(3)],
            groups: vec![
                (GroupId(10), SectionId(1)),
                (GroupId(11), SectionId(1)),
                (GroupId(12), SectionId(1)),
                (GroupId(20), SectionId(2)),
            ],
            notes: vec![(NoteId(100), GroupId(10)), (NoteId(101), GroupId(11))],
            labels: vec![],
        }
    }
    fn root() -> Rectangle {
        Rectangle::new(Point::ORIGIN, Size::new(1000.0, 800.0))
    }
    fn row(kind: RowKind, y: f32) -> RowGeometry {
        RowGeometry::clipped(
            kind,
            Rectangle::new(Point::new(20.0, y), Size::new(180.0, 36.0)),
            root(),
        )
        .unwrap()
    }
    fn geometry() -> Geometry {
        Geometry {
            root: root(),
            rows: vec![
                row(RowKind::Note(NoteId(100)), 0.0),
                row(RowKind::Group(GroupId(20)), 50.0),
                row(RowKind::Section(SectionId(2)), 100.0),
            ],
        }
    }
    fn started() -> Drag {
        let mut drag = Drag::default();
        assert!(drag.press(
            RowKind::Note(NoteId(100)),
            Point::new(30.0, 10.0),
            7,
            &catalog()
        ));
        drag
    }
    fn commits(events: &[DragEvent]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, DragEvent::Commit { .. }))
            .count()
    }

    #[test]
    fn threshold_is_inclusive_at_four_not_three_point_nine() {
        let mut drag = started();
        assert!(drag
            .motion(
                Point::new(33.9, 10.0),
                Duration::ZERO,
                7,
                &catalog(),
                &geometry()
            )
            .is_empty());
        assert!(!drag.is_dragging());
        drag.motion(
            Point::new(34.0, 10.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        assert!(drag.is_dragging());
        drag.motion(
            Point::new(30.0, 10.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        assert!(drag.is_dragging(), "returning to origin is still a drag");
    }

    #[test]
    fn every_allowed_move_pairing_has_the_exact_op() {
        let source = RowKind::Note(NoteId(100));
        let hit = row(RowKind::Group(GroupId(20)), 0.0);
        assert_eq!(
            target(source, &hit, hit.bounds.center(), &catalog())
                .unwrap()
                .action,
            DropAction::MoveNote {
                note: NoteId(100),
                to: GroupId(20),
            }
        );
        let hit = row(RowKind::Section(SectionId(2)), 0.0);
        assert_eq!(
            target(
                RowKind::Group(GroupId(10)),
                &hit,
                hit.bounds.center(),
                &catalog()
            )
            .unwrap()
            .action,
            DropAction::MoveGroup {
                group: GroupId(10),
                to: SectionId(2)
            }
        );
    }

    #[test]
    fn all_source_destination_type_pairs_are_restricted_to_the_spec() {
        let sources = [
            RowKind::Section(SectionId(1)),
            RowKind::Group(GroupId(10)),
            RowKind::Note(NoteId(100)),
        ];
        let destinations = [
            RowKind::Section(SectionId(3)),
            RowKind::Group(GroupId(12)),
            RowKind::Note(NoteId(101)),
        ];
        // A note drops only on a group: notes never live directly in a section.
        let allowed = [
            [true, false, false],
            [true, true, false],
            [false, true, false],
        ];
        for (i, source) in sources.into_iter().enumerate() {
            for (j, destination) in destinations.into_iter().enumerate() {
                let hit = row(destination, 0.0);
                let at = Point::new(30.0, 35.0);
                assert_eq!(
                    target(source, &hit, at, &catalog()).is_some(),
                    allowed[i][j],
                    "{source:?} -> {destination:?}"
                );
            }
        }
    }

    #[test]
    fn groups_reorder_only_among_their_own_siblings_before_and_after() {
        let source = RowKind::Group(GroupId(10));
        let hit = row(RowKind::Group(GroupId(12)), 0.0);
        let before = target(source, &hit, Point::new(30.0, 1.0), &catalog()).unwrap();
        assert_eq!(
            before.action,
            DropAction::Reorder {
                target: ItemRef::Group(GroupId(10)),
                before: Some(ItemRef::Group(GroupId(12)))
            }
        );
        assert_eq!(before.insertion.unwrap().edge, Edge::Before);
        let after = target(source, &hit, Point::new(30.0, 35.0), &catalog()).unwrap();
        assert_eq!(
            after.action,
            DropAction::Reorder {
                target: ItemRef::Group(GroupId(10)),
                before: None
            }
        );
        assert_eq!(after.insertion.unwrap().edge, Edge::After);
        assert!(target(
            source,
            &row(RowKind::Group(GroupId(20)), 0.0),
            Point::new(30.0, 1.0),
            &catalog()
        )
        .is_none());
    }

    #[test]
    fn section_insertion_before_after_and_identity_noops() {
        let source = RowKind::Section(SectionId(1));
        let hit = row(RowKind::Section(SectionId(3)), 0.0);
        assert_eq!(
            target(source, &hit, Point::new(30.0, 0.0), &catalog())
                .unwrap()
                .action,
            DropAction::Reorder {
                target: ItemRef::Section(SectionId(1)),
                before: Some(ItemRef::Section(SectionId(3)))
            }
        );
        assert_eq!(
            target(source, &hit, Point::new(30.0, 35.0), &catalog())
                .unwrap()
                .action,
            DropAction::Reorder {
                target: ItemRef::Section(SectionId(1)),
                before: None
            }
        );
        assert!(target(
            source,
            &row(RowKind::Section(SectionId(2)), 0.0),
            Point::new(30.0, 1.0),
            &catalog()
        )
        .is_none());
        assert!(target(
            source,
            &row(source, 0.0),
            Point::new(30.0, 35.0),
            &catalog()
        )
        .is_none());
    }

    #[test]
    fn same_location_moves_missing_items_and_note_to_section_drops_are_invalid() {
        let catalog = catalog();
        for (source, destination) in [
            (RowKind::Note(NoteId(100)), RowKind::Group(GroupId(10))),
            (RowKind::Note(NoteId(101)), RowKind::Group(GroupId(11))),
            (RowKind::Note(NoteId(100)), RowKind::Section(SectionId(1))),
            (RowKind::Note(NoteId(100)), RowKind::Section(SectionId(2))),
            (RowKind::Group(GroupId(10)), RowKind::Section(SectionId(1))),
            (RowKind::Note(NoteId(999)), RowKind::Group(GroupId(20))),
            (RowKind::Note(NoteId(100)), RowKind::Group(GroupId(999))),
        ] {
            let hit = row(destination, 0.0);
            assert!(
                target(source, &hit, hit.bounds.center(), &catalog).is_none(),
                "{source:?} -> {destination:?}"
            );
        }
    }

    #[test]
    fn clipped_rows_never_hit_headers_or_outside_the_root() {
        let viewport = Rectangle::new(Point::new(20.0, 100.0), Size::new(180.0, 100.0));
        let full = Rectangle::new(Point::new(20.0, 80.0), Size::new(180.0, 36.0));
        let clipped = RowGeometry::clipped(RowKind::Group(GroupId(20)), full, viewport).unwrap();
        assert_eq!(clipped.visible.y, 100.0);
        assert_eq!(
            clipped.bounds.center_y(),
            98.0,
            "clipping preserves the real row center"
        );
        let geometry = Geometry {
            root: root(),
            rows: vec![clipped],
        };
        assert!(geometry.hit(Point::new(30.0, 90.0)).is_none());
        assert!(geometry.hit(Point::new(30.0, 105.0)).is_some());
        assert!(geometry.hit(Point::new(-1.0, 105.0)).is_none());
        assert!(RowGeometry::clipped(
            RowKind::Group(GroupId(20)),
            full,
            Rectangle::new(Point::new(20.0, 200.0), Size::new(180.0, 100.0))
        )
        .is_none());
    }

    #[test]
    fn spring_fires_at_six_hundred_once_and_never_opens_a_note() {
        let mut drag = started();
        let position = Point::new(30.0, 110.0);
        drag.motion(position, Duration::ZERO, 7, &catalog(), &geometry());
        assert_eq!(drag.next_deadline(), Some(SPRING_DELAY));
        let early = drag.refresh(Duration::from_millis(599), 7, &catalog(), &geometry());
        assert!(!early
            .iter()
            .any(|event| matches!(event, DragEvent::Spring { .. })));
        let due = drag.refresh(Duration::from_millis(600), 7, &catalog(), &geometry());
        assert_eq!(
            due.iter()
                .filter(|event| matches!(
                    event,
                    DragEvent::Spring {
                        section: SectionId(2),
                        ..
                    }
                ))
                .count(),
            1
        );
        assert_eq!(commits(&due), 0);
        assert_eq!(drag.next_deadline(), None);
        assert!(drag
            .refresh(Duration::from_secs(3), 7, &catalog(), &geometry())
            .is_empty());
        assert!(
            drag.preview().unwrap().target.is_none(),
            "a section springs open for a note but is never its drop target"
        );
        let events = drag.release(
            Some(position),
            Duration::from_secs(3),
            7,
            &catalog(),
            &geometry(),
        );
        assert!(matches!(
            events.as_slice(),
            [DragEvent::Cancelled {
                reason: CancelReason::InvalidTarget,
                ..
            }]
        ));
    }

    #[test]
    fn leaving_section_restarts_its_hover_timer() {
        let mut drag = started();
        drag.motion(
            Point::new(30.0, 110.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        drag.motion(
            Point::new(30.0, 60.0),
            Duration::from_millis(500),
            7,
            &catalog(),
            &geometry(),
        );
        assert_eq!(drag.next_deadline(), None);
        drag.motion(
            Point::new(30.0, 110.0),
            Duration::from_millis(550),
            7,
            &catalog(),
            &geometry(),
        );
        assert_eq!(drag.next_deadline(), Some(Duration::from_millis(1150)));
    }

    #[test]
    fn spring_can_hide_source_and_drop_still_commits_exactly_once() {
        let mut drag = started();
        drag.motion(
            Point::new(30.0, 110.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        drag.refresh(SPRING_DELAY, 7, &catalog(), &geometry());
        let mut hidden = geometry();
        hidden
            .rows
            .retain(|row| row.row != RowKind::Note(NoteId(100)));
        drag.motion(
            Point::new(30.0, 60.0),
            Duration::from_millis(700),
            7,
            &catalog(),
            &hidden,
        );
        let events = drag.release(
            Some(Point::new(30.0, 60.0)),
            Duration::from_millis(700),
            7,
            &catalog(),
            &hidden,
        );
        assert_eq!(commits(&events), 1);
        assert!(events.iter().any(|event| matches!(
            event,
            DragEvent::Commit {
                op: Op::MoveNote {
                    note: NoteId(100),
                    to: GroupId(20)
                },
                ..
            }
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, DragEvent::Activate { .. })));
        assert!(drag
            .release(
                Some(Point::new(30.0, 60.0)),
                Duration::from_secs(1),
                7,
                &catalog(),
                &hidden
            )
            .is_empty());
    }

    #[test]
    fn source_deletion_or_parent_change_cancels_not_hiding() {
        let mut drag = started();
        let mut removed = catalog();
        removed.notes.retain(|(id, _)| *id != NoteId(100));
        assert!(matches!(
            drag.validate(7, &removed),
            Some(DragEvent::Cancelled {
                reason: CancelReason::SourceRemoved,
                ..
            })
        ));
        let mut changed = catalog();
        changed.notes[0].1 = GroupId(20);
        let mut drag = started();
        assert!(matches!(
            drag.validate(7, &changed),
            Some(DragEvent::Cancelled {
                reason: CancelReason::SourceChanged,
                ..
            })
        ));
    }

    fn reorder_started() -> (Drag, Catalog, Geometry) {
        let mut catalog = catalog();
        catalog.groups.insert(3, (GroupId(13), SectionId(1)));
        let geometry = Geometry {
            root: root(),
            rows: vec![
                row(RowKind::Group(GroupId(10)), 0.0),
                row(RowKind::Group(GroupId(13)), 50.0),
            ],
        };
        let mut drag = Drag::default();
        assert!(drag.press(
            RowKind::Group(GroupId(13)),
            Point::new(30.0, 60.0),
            7,
            &catalog,
        ));
        drag.motion(
            Point::new(30.0, 35.0),
            Duration::ZERO,
            7,
            &catalog,
            &geometry,
        );
        assert_eq!(
            drag.preview().unwrap().target.unwrap().action,
            DropAction::Reorder {
                target: ItemRef::Group(GroupId(13)),
                before: Some(ItemRef::Group(GroupId(11))),
            }
        );
        (drag, catalog, geometry)
    }

    fn assert_target_change_cancels(
        drag: Drag,
        changed: &Catalog,
        geometry: &Geometry,
        expected_reason: CancelReason,
    ) {
        let position = drag.preview().unwrap().position;
        for step in ["validate", "refresh", "motion", "release"] {
            let mut drag = drag.clone();
            let events = match step {
                "validate" => drag.validate(7, changed).into_iter().collect(),
                "refresh" => drag.refresh(Duration::ZERO, 7, changed, geometry),
                "motion" => drag.motion(position, Duration::ZERO, 7, changed, geometry),
                "release" => drag.release(Some(position), Duration::ZERO, 7, changed, geometry),
                _ => unreachable!(),
            };
            assert!(
                matches!(events.as_slice(), [DragEvent::Cancelled { reason, .. }] if *reason == expected_reason),
                "{step} must cancel the prior target, not retarget: {events:?}",
            );
            assert_eq!(commits(&events), 0);
            assert!(!drag.is_active(), "{step} retained the invalid gesture");
            assert!(drag.preview().is_none());
        }
    }

    #[test]
    fn removed_hidden_reorder_anchor_cancels_before_any_retarget() {
        let (drag, mut catalog, geometry) = reorder_started();
        catalog.groups.retain(|(id, _)| *id != GroupId(11));
        assert_target_change_cancels(drag, &catalog, &geometry, CancelReason::TargetRemoved);
    }

    #[test]
    fn moved_hidden_reorder_anchor_cancels_before_any_retarget() {
        let (drag, mut catalog, geometry) = reorder_started();
        catalog.groups[1].1 = SectionId(2);
        assert_target_change_cancels(drag, &catalog, &geometry, CancelReason::TargetChanged);
    }

    #[test]
    fn removed_hovered_destination_cancels_before_any_retarget() {
        let mut drag = started();
        drag.motion(
            Point::new(30.0, 60.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        let mut changed = catalog();
        changed.groups.retain(|(id, _)| *id != GroupId(20));
        assert_target_change_cancels(drag, &changed, &geometry(), CancelReason::TargetRemoved);
    }

    #[test]
    fn moved_hovered_destination_cancels_before_any_retarget() {
        let mut drag = started();
        drag.motion(
            Point::new(30.0, 60.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        let mut changed = catalog();
        changed.groups.last_mut().unwrap().1 = SectionId(3);
        assert_target_change_cancels(drag, &changed, &geometry(), CancelReason::TargetChanged);
    }

    #[test]
    fn removed_section_destination_cancels() {
        let geometry = Geometry {
            root: root(),
            rows: vec![row(RowKind::Section(SectionId(2)), 50.0)],
        };
        let mut drag = Drag::default();
        assert!(drag.press(
            RowKind::Group(GroupId(10)),
            Point::new(30.0, 10.0),
            7,
            &catalog()
        ));
        drag.motion(
            Point::new(30.0, 60.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry,
        );
        assert_eq!(
            drag.preview().unwrap().target.unwrap().action,
            DropAction::MoveGroup {
                group: GroupId(10),
                to: SectionId(2)
            }
        );
        let mut changed = catalog();
        changed.sections.retain(|id| *id != SectionId(2));
        assert_target_change_cancels(drag, &changed, &geometry, CancelReason::TargetRemoved);
    }

    #[test]
    fn hiding_target_geometry_does_not_remove_model_references() {
        let (mut drag, catalog, geometry) = reorder_started();
        let hidden = Geometry::new(root());
        let events = drag.refresh(Duration::ZERO, 7, &catalog, &hidden);
        assert!(!events
            .iter()
            .any(|event| matches!(event, DragEvent::Cancelled { .. })));
        assert!(drag.is_active());
        assert!(drag.preview().unwrap().target.is_none());
        let events = drag.release(
            Some(Point::new(30.0, 35.0)),
            Duration::ZERO,
            7,
            &catalog,
            &geometry,
        );
        assert_eq!(commits(&events), 1);
        assert!(events.iter().any(|event| matches!(
            event,
            DragEvent::Commit {
                op: Op::Reorder {
                    target: ItemRef::Group(GroupId(13)),
                    before: Some(ItemRef::Group(GroupId(11)))
                },
                ..
            }
        )));
    }

    #[test]
    fn ordinary_sibling_additions_and_reorders_do_not_invalidate_references() {
        let (mut drag, mut catalog, geometry) = reorder_started();
        catalog.groups.insert(1, (GroupId(14), SectionId(1)));
        catalog.groups.swap(2, 3);
        let events = drag.refresh(Duration::ZERO, 7, &catalog, &geometry);
        assert!(!events
            .iter()
            .any(|event| matches!(event, DragEvent::Cancelled { .. })));
        assert!(drag.is_active());
        assert_eq!(
            drag.preview().unwrap().target.unwrap().action,
            DropAction::Reorder {
                target: ItemRef::Group(GroupId(13)),
                before: Some(ItemRef::Group(GroupId(14))),
            }
        );
        let events = drag.release(
            Some(Point::new(30.0, 35.0)),
            Duration::ZERO,
            7,
            &catalog,
            &geometry,
        );
        assert_eq!(commits(&events), 1);
    }

    #[test]
    fn ordinary_pointer_motion_can_select_another_model_valid_destination() {
        let mut geometry = geometry();
        geometry.rows.push(row(RowKind::Group(GroupId(11)), 150.0));
        let mut drag = started();
        drag.motion(
            Point::new(30.0, 60.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry,
        );
        let events = drag.motion(
            Point::new(30.0, 160.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry,
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event, DragEvent::Cancelled { .. })));
        let events = drag.release(
            Some(Point::new(30.0, 160.0)),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry,
        );
        assert_eq!(commits(&events), 1);
        assert!(events.iter().any(|event| matches!(
            event,
            DragEvent::Commit {
                op: Op::MoveNote {
                    note: NoteId(100),
                    to: GroupId(11)
                },
                ..
            }
        )));
    }

    #[test]
    fn stale_generation_and_every_explicit_cancel_suppress_the_later_release() {
        for reason in [
            CancelReason::Escape,
            CancelReason::Blur,
            CancelReason::Disabled,
        ] {
            let mut drag = started();
            drag.motion(
                Point::new(30.0, 60.0),
                Duration::ZERO,
                7,
                &catalog(),
                &geometry(),
            );
            assert!(drag.cancel(reason).is_some());
            assert!(
                drag.is_pressed(),
                "the root must still own the orphan release"
            );
            assert!(drag
                .release(
                    Some(Point::new(30.0, 60.0)),
                    Duration::ZERO,
                    7,
                    &catalog(),
                    &geometry()
                )
                .is_empty());
            assert!(!drag.is_pressed());
        }
        let mut drag = started();
        let events = drag.release(
            Some(Point::new(30.0, 60.0)),
            Duration::ZERO,
            8,
            &catalog(),
            &geometry(),
        );
        assert!(matches!(
            events.as_slice(),
            [DragEvent::Cancelled {
                generation: 7,
                reason: CancelReason::StaleGeneration
            }]
        ));
    }

    #[test]
    fn retiring_a_cancelled_release_never_retires_an_active_gesture() {
        let mut drag = started();
        drag.retire_cancelled_release();
        assert!(drag.is_active());
        assert!(drag.is_pressed());
        let events = drag.release(
            Some(Point::new(30.0, 10.0)),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        assert!(matches!(events.as_slice(), [DragEvent::Activate { .. }]));

        let mut drag = started();
        drag.cancel(CancelReason::Blur);
        drag.retire_cancelled_release();
        assert!(!drag.is_pressed());
        assert!(drag
            .release(
                Some(Point::new(30.0, 60.0)),
                Duration::ZERO,
                7,
                &catalog(),
                &geometry()
            )
            .is_empty());
    }

    #[test]
    fn nondrag_release_activates_only_the_original_row() {
        let mut drag = started();
        let events = drag.release(
            Some(Point::new(33.9, 10.0)),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        assert!(matches!(
            events.as_slice(),
            [DragEvent::Activate {
                row: RowKind::Note(NoteId(100)),
                ..
            }]
        ));
        let mut drag = started();
        let events = drag.release(None, Duration::ZERO, 7, &catalog(), &geometry());
        assert_eq!(commits(&events), 0);
        assert!(!events
            .iter()
            .any(|event| matches!(event, DragEvent::Activate { .. })));
    }

    fn click(
        drag: &mut Drag,
        row: RowKind,
        at: Point,
        now: Duration,
        generation: u64,
        catalog: &Catalog,
        geometry: &Geometry,
    ) -> Vec<DragEvent> {
        assert!(drag.press(row, at, generation, catalog));
        drag.release(Some(at), now, generation, catalog, geometry)
    }

    #[test]
    fn double_click_open_uses_the_owned_release_and_exact_stable_row() {
        for kind in [
            RowKind::Note(NoteId(100)),
            RowKind::Group(GroupId(10)),
            RowKind::Section(SectionId(1)),
        ] {
            let geometry = Geometry {
                root: root(),
                rows: vec![row(kind, 0.0)],
            };
            let mut drag = Drag::default();
            let at = Point::new(30.0, 10.0);
            assert!(
                matches!(click(&mut drag, kind, at, Duration::ZERO, 7, &catalog(), &geometry).as_slice(), [DragEvent::Activate { row, .. }] if *row == kind)
            );
            assert!(
                matches!(click(&mut drag, kind, Point::new(33.9,10.0), DOUBLE_CLICK_DELAY, 7, &catalog(), &geometry).as_slice(), [DragEvent::Open { generation: 7, row }] if *row == kind)
            );
            assert!(!drag.is_pressed());
            assert!(matches!(
                click(
                    &mut drag,
                    kind,
                    at,
                    DOUBLE_CLICK_DELAY,
                    7,
                    &catalog(),
                    &geometry
                )
                .as_slice(),
                [DragEvent::Activate { .. }]
            ));
        }
    }

    #[test]
    fn click_timeout_or_four_pixel_separation_never_opens() {
        let kind = RowKind::Note(NoteId(100));
        for (at, delay) in [
            (Point::new(34.0, 10.0), Duration::from_millis(100)),
            (
                Point::new(30.0, 10.0),
                DOUBLE_CLICK_DELAY + Duration::from_millis(1),
            ),
        ] {
            let mut drag = Drag::default();
            click(
                &mut drag,
                kind,
                Point::new(30.0, 10.0),
                Duration::ZERO,
                7,
                &catalog(),
                &geometry(),
            );
            assert!(matches!(
                click(&mut drag, kind, at, delay, 7, &catalog(), &geometry()).as_slice(),
                [DragEvent::Activate { .. }]
            ));
        }
    }

    #[test]
    fn second_press_crossing_the_drag_threshold_never_becomes_an_open() {
        let kind = RowKind::Note(NoteId(100));
        let at = Point::new(30.0, 10.0);
        let mut drag = Drag::default();
        click(
            &mut drag,
            kind,
            at,
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        assert!(drag.press(kind, at, 7, &catalog()));
        drag.motion(
            Point::new(34.0, 10.0),
            Duration::from_millis(100),
            7,
            &catalog(),
            &geometry(),
        );
        let events = drag.release(
            Some(at),
            Duration::from_millis(100),
            7,
            &catalog(),
            &geometry(),
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event, DragEvent::Open { .. } | DragEvent::Activate { .. })));
        assert!(matches!(
            click(
                &mut drag,
                kind,
                at,
                Duration::from_millis(200),
                7,
                &catalog(),
                &geometry()
            )
            .as_slice(),
            [DragEvent::Activate { .. }]
        ));
    }

    #[test]
    fn click_history_is_invalidated_by_cancel_generation_parent_change_and_hidden_source() {
        let kind = RowKind::Note(NoteId(100));
        let at = Point::new(30.0, 10.0);
        let mut first = Drag::default();
        click(
            &mut first,
            kind,
            at,
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        for reason in [
            CancelReason::Escape,
            CancelReason::Blur,
            CancelReason::Disabled,
        ] {
            let mut drag = first.clone();
            drag.cancel(reason);
            assert!(matches!(
                click(
                    &mut drag,
                    kind,
                    at,
                    Duration::from_millis(100),
                    7,
                    &catalog(),
                    &geometry()
                )
                .as_slice(),
                [DragEvent::Activate { .. }]
            ));
        }
        let mut drag = first.clone();
        assert!(matches!(
            click(
                &mut drag,
                kind,
                at,
                Duration::from_millis(100),
                8,
                &catalog(),
                &geometry()
            )
            .as_slice(),
            [DragEvent::Activate { .. }]
        ));
        let mut changed = catalog();
        changed.notes[0].1 = GroupId(20);
        let mut drag = first.clone();
        assert!(matches!(
            click(
                &mut drag,
                kind,
                at,
                Duration::from_millis(100),
                7,
                &changed,
                &geometry()
            )
            .as_slice(),
            [DragEvent::Activate { .. }]
        ));
        let mut drag = first;
        drag.refresh(
            Duration::from_millis(50),
            7,
            &catalog(),
            &Geometry::new(root()),
        );
        assert!(matches!(
            click(
                &mut drag,
                kind,
                at,
                Duration::from_millis(100),
                7,
                &catalog(),
                &geometry()
            )
            .as_slice(),
            [DragEvent::Activate { .. }]
        ));
    }

    #[test]
    fn invalid_drop_and_pointer_exit_never_commit_or_activate() {
        let mut drag = started();
        drag.motion(
            Point::new(30.0, 60.0),
            Duration::ZERO,
            7,
            &catalog(),
            &geometry(),
        );
        drag.pointer_left(Duration::from_secs(1), 7, &catalog(), &geometry());
        assert!(drag.preview().unwrap().target.is_none());
        let events = drag.release(None, Duration::from_secs(1), 7, &catalog(), &geometry());
        assert!(matches!(
            events.as_slice(),
            [DragEvent::Cancelled {
                reason: CancelReason::InvalidTarget,
                ..
            }]
        ));
    }
}
