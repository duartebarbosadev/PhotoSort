//! Pointer gestures shared by folder dragging and photo panning. A complete
//! gesture may arrive between repaints, after egui's `Response::dragged` ends.
use eframe::egui::{self, Pos2, Vec2};

#[derive(Clone, Copy)]
pub(super) struct Motion {
    pub start: Pos2,
    pub end: Pos2,
    delta: Vec2,
    finished: bool,
    claimed: bool,
}
struct Active {
    start: Pos2,
    current: Pos2,
    applied: Pos2,
    dragging: bool,
}
#[derive(Default)]
pub(super) struct DragTracker {
    frame: Option<(u64, egui::Id)>,
    active: Option<Active>,
    motions: Vec<Motion>,
    pub cancelled: bool,
}
impl DragTracker {
    /// Returns false on repeated layout passes; callers must not repeat commands.
    pub fn begin_frame(&mut self, ctx: &egui::Context, context: egui::Id) -> bool {
        let frame = ctx.cumulative_frame_nr();
        if self.frame == Some((frame, context)) {
            return false;
        }
        self.cancelled = self.frame.is_some_and(|(previous, owner)| {
            owner != context || previous.saturating_add(1) < frame
        }) && self.active.take().is_some();
        self.frame = Some((frame, context));
        self.motions.clear();
        self.cancelled |= ctx.input(|input| {
            !input.raw.focused
                || input.key_pressed(egui::Key::Escape)
                || input.raw.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            pressed: true,
                            ..
                        } | egui::Event::WindowFocused(false)
                    )
                })
        });
        if self.cancelled {
            self.active = None;
            return true;
        }
        ctx.input(|input| {
            for event in &input.raw.events {
                match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        ..
                    } => {
                        if *pressed {
                            self.active = Some(Active {
                                start: *pos,
                                current: *pos,
                                applied: *pos,
                                dragging: false,
                            });
                        } else if let Some(active) = self.active.as_mut() {
                            active.current = *pos;
                            self.emit(true);
                            self.active = None;
                        }
                    }
                    egui::Event::PointerMoved(pos) => {
                        if let Some(active) = self.active.as_mut() {
                            active.current = *pos;
                            active.dragging |= active.start.distance(*pos) > 6.0;
                        }
                    }
                    egui::Event::PointerGone => {
                        self.cancelled |= self.active.take().is_some();
                    }
                    _ => {}
                }
            }
        });
        self.emit(false);
        true
    }
    fn emit(&mut self, finished: bool) {
        if let Some(active) = self.active.as_mut() {
            active.dragging |= active.start.distance(active.current) > 6.0;
            if active.dragging {
                self.motions.push(Motion {
                    start: active.start,
                    end: active.current,
                    delta: active.current - active.applied,
                    finished,
                    claimed: false,
                });
                active.applied = active.current;
            }
        }
    }
    pub fn start(&self) -> Option<Pos2> {
        self.active.as_ref().map(|active| active.start)
    }
    pub fn completed(&self) -> Option<Motion> {
        self.motions
            .iter()
            .rev()
            .find(|motion| motion.finished)
            .copied()
    }
    /// Claim each movement once, using the press location rather than the latest
    /// hover target. This keeps fast drags anchored to the correct comparison pane.
    pub fn take_pan_delta(&mut self, ui: &egui::Ui, response: &egui::Response) -> Option<Vec2> {
        let mut delta = None;
        for motion in &mut self.motions {
            if !motion.claimed && contains_pointer(ui, response, motion.start) {
                *delta.get_or_insert(Vec2::ZERO) += motion.delta;
                motion.claimed = true;
            }
        }
        delta
    }
}
pub(super) fn contains_pointer(ui: &egui::Ui, response: &egui::Response, pos: Pos2) -> bool {
    response.enabled()
        && response.rect.intersect(ui.clip_rect()).contains(pos)
        && ui.ctx().layer_id_at(pos) == Some(response.layer_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        }
    }
    fn frame(
        ctx: &egui::Context,
        tracker: &mut DragTracker,
        owner: u64,
        focused: bool,
        events: Vec<egui::Event>,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                focused,
                events,
                ..Default::default()
            },
            |ctx| {
                tracker.begin_frame(ctx, egui::Id::new(owner));
            },
        );
    }
    #[test]
    fn cancelled_or_abandoned_gestures_cannot_resume_in_another_context() {
        for reason in 0..6 {
            let ctx = egui::Context::default();
            crate::ui::theme::install(&ctx);
            let mut tracker = DragTracker::default();
            let start = egui::pos2(20.0, 20.0);
            let end = egui::pos2(80.0, 90.0);
            frame(&ctx, &mut tracker, 1, true, vec![button(start, true)]);
            match reason {
                0 => frame(
                    &ctx,
                    &mut tracker,
                    1,
                    true,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                ),
                1 => frame(&ctx, &mut tracker, 1, false, vec![]),
                2 => frame(&ctx, &mut tracker, 1, true, vec![egui::Event::PointerGone]),
                3 => frame(&ctx, &mut tracker, 2, true, vec![]),
                4 => {
                    let _ = ctx.run(Default::default(), |_| {});
                }
                _ => frame(
                    &ctx,
                    &mut tracker,
                    1,
                    true,
                    vec![
                        egui::Event::WindowFocused(false),
                        egui::Event::WindowFocused(true),
                    ],
                ),
            }
            frame(
                &ctx,
                &mut tracker,
                if reason == 3 { 2 } else { 1 },
                true,
                vec![egui::Event::PointerMoved(end), button(end, false)],
            );
            assert!(tracker.completed().is_none(), "reason {reason}");
            assert!(tracker.motions.is_empty(), "reason {reason}");
        }
    }
    #[test]
    fn coalesced_drags_are_claimed_by_the_press_pane_once_and_respect_clipping() {
        for clipped in [false, true] {
            let ctx = egui::Context::default();
            crate::ui::theme::install(&ctx);
            let mut tracker = DragTracker::default();
            let mut rectangles = Vec::new();
            let mut render = |events, tracker: &mut DragTracker| {
                let mut pans = Vec::new();
                rectangles.clear();
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(500., 300.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        tracker.begin_frame(ctx, egui::Id::new("panes"));
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                for pane in 0..2 {
                                    let (_, response) = ui.allocate_exact_size(
                                        egui::vec2(200., 200.),
                                        egui::Sense::click_and_drag(),
                                    );
                                    rectangles.push(response.rect);
                                    let original_clip = ui.clip_rect();
                                    if clipped && pane == 0 {
                                        ui.set_clip_rect(egui::Rect::NOTHING);
                                    }
                                    pans.push(tracker.take_pan_delta(ui, &response));
                                    assert!(
                                        tracker.take_pan_delta(ui, &response).is_none(),
                                        "same-frame movement repeated"
                                    );
                                    ui.set_clip_rect(original_clip);
                                }
                            });
                        });
                    },
                );
                (rectangles.clone(), pans)
            };
            let mut rects = Vec::new();
            for _ in 0..3 {
                rects = render(vec![], &mut tracker).0;
            }
            let start = rects[0].center();
            let end = rects[1].center();
            let (_, pans) = render(
                vec![
                    egui::Event::PointerMoved(start),
                    button(start, true),
                    egui::Event::PointerMoved(end),
                    button(end, false),
                ],
                &mut tracker,
            );
            assert_eq!(pans[0], (!clipped).then_some(end - start));
            assert_eq!(pans[1], None, "latest hover pane stole the drag");
            assert_eq!(render(vec![], &mut tracker).1, [None, None]);
        }
    }
    #[test]
    fn clicks_and_secondary_buttons_do_not_pan_and_multiple_drags_keep_their_deltas() {
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        let mut tracker = DragTracker::default();
        let start = egui::pos2(20., 20.);
        let end = egui::pos2(80., 90.);
        frame(
            &ctx,
            &mut tracker,
            1,
            true,
            vec![
                button(start, true),
                button(start + egui::vec2(2., 1.), false),
            ],
        );
        assert!(tracker.motions.is_empty());
        frame(
            &ctx,
            &mut tracker,
            1,
            true,
            vec![
                egui::Event::PointerButton {
                    pos: start,
                    pressed: true,
                    button: egui::PointerButton::Secondary,
                    modifiers: Default::default(),
                },
                egui::Event::PointerMoved(end),
            ],
        );
        assert!(tracker.motions.is_empty());
        frame(
            &ctx,
            &mut tracker,
            1,
            true,
            vec![
                button(start, true),
                button(end, false),
                button(end, true),
                button(start, false),
            ],
        );
        assert_eq!(tracker.motions.len(), 2);
        assert_eq!(tracker.motions[0].delta, end - start);
        assert_eq!(tracker.motions[1].delta, start - end);
    }
}
