use iced::advanced::widget::operation::{self, Outcome};
use iced::advanced::widget::{Id, Operation};
use iced::{Rectangle, Vector};

pub fn ensure_visible(scroll: Id, row: Id) -> impl Operation<()> {
    Locate {
        scroll,
        row,
        viewport: None,
        content: None,
        translation: Vector::ZERO,
        bounds: None,
    }
}

struct Locate {
    scroll: Id,
    row: Id,
    viewport: Option<Rectangle>,
    content: Option<Rectangle>,
    translation: Vector,
    bounds: Option<Rectangle>,
}

impl Operation<()> for Locate {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        _: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&self.scroll) {
            self.viewport = Some(bounds);
            self.content = Some(content);
            self.translation = translation;
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.row) {
            self.bounds = Some(bounds);
        }
    }

    fn finish(&self) -> Outcome<()> {
        let (Some(viewport), Some(content), Some(row)) = (self.viewport, self.content, self.bounds)
        else {
            return Outcome::Some(());
        };
        match minimum_reveal(viewport, content, row, self.translation.y) {
            Some(offset) => Outcome::Chain(Box::new(Apply {
                scroll: self.scroll.clone(),
                offset,
            })),
            None => Outcome::Some(()),
        }
    }
}

struct Apply {
    scroll: Id,
    offset: f32,
}

impl Operation<()> for Apply {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _: Rectangle,
        _: Rectangle,
        _: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&self.scroll) {
            state.scroll_to(operation::scrollable::AbsoluteOffset {
                x: None,
                y: Some(self.offset),
            });
        }
    }
    fn finish(&self) -> Outcome<()> {
        Outcome::Some(())
    }
}

// Headers and search snippets have different heights. Reveal the actual row's
// nearest clipped edge; an equal-row snap can hide a variable-height result.
fn minimum_reveal(
    viewport: Rectangle,
    content: Rectangle,
    row: Rectangle,
    offset: f32,
) -> Option<f32> {
    if viewport.height <= 0.0 || row.height <= 0.0 {
        return None;
    }
    let leading = row.y - offset;
    let trailing = leading + row.height;
    let desired = if leading < viewport.y - 0.5 {
        row.y - content.y
    } else if trailing > viewport.y + viewport.height + 0.5 {
        if row.height > viewport.height {
            row.y - content.y
        } else {
            row.y + row.height - content.y - viewport.height
        }
    } else {
        return None;
    };
    Some(desired.clamp(0.0, (content.height - viewport.height).max(0.0)))
}

#[cfg(feature = "smoke-test")]
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub viewport: Rectangle,
    pub content: Rectangle,
    pub row: Rectangle,
    pub translation: Vector,
}

#[cfg(feature = "smoke-test")]
pub fn measure(scroll: Id, row: Id) -> impl Operation<Option<Geometry>> {
    Measure(Locate {
        scroll,
        row,
        viewport: None,
        content: None,
        translation: Vector::ZERO,
        bounds: None,
    })
}

#[cfg(feature = "smoke-test")]
struct Measure(Locate);
#[cfg(feature = "smoke-test")]
impl Operation<Option<Geometry>> for Measure {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<Geometry>>)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        _: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&self.0.scroll) {
            self.0.viewport = Some(bounds);
            self.0.content = Some(content);
            self.0.translation = translation;
        }
    }
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.0.row) {
            self.0.bounds = Some(bounds);
        }
    }
    fn finish(&self) -> Outcome<Option<Geometry>> {
        Outcome::Some(match (self.0.viewport, self.0.content, self.0.bounds) {
            (Some(viewport), Some(content), Some(row)) => Some(Geometry {
                viewport,
                content,
                row,
                translation: self.0.translation,
            }),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds(y: f32, height: f32) -> Rectangle {
        Rectangle {
            x: 0.0,
            y,
            width: 500.0,
            height,
        }
    }
    #[test]
    fn variable_height_rows_reveal_only_the_missing_edge() {
        assert_eq!(
            minimum_reveal(
                bounds(96.0, 400.0),
                bounds(96.0, 1200.0),
                bounds(620.0, 76.0),
                0.0
            ),
            Some(200.0)
        );
        assert_eq!(
            minimum_reveal(
                bounds(96.0, 400.0),
                bounds(96.0, 1200.0),
                bounds(240.0, 44.0),
                200.0
            ),
            Some(144.0)
        );
        assert_eq!(
            minimum_reveal(
                bounds(96.0, 400.0),
                bounds(96.0, 1200.0),
                bounds(240.0, 76.0),
                100.0
            ),
            None
        );
    }
    #[test]
    fn oversized_row_aligns_its_top_and_short_content_never_goes_negative() {
        assert_eq!(
            minimum_reveal(
                bounds(20.0, 50.0),
                bounds(20.0, 500.0),
                bounds(120.0, 80.0),
                0.0
            ),
            Some(100.0)
        );
        assert_eq!(
            minimum_reveal(
                bounds(20.0, 500.0),
                bounds(20.0, 200.0),
                bounds(10.0, 44.0),
                0.0
            ),
            Some(0.0)
        );
    }
}
