//! Focus and scrolling for virtualized file lists and navigation trees.
use super::*;

pub(super) struct Keyboard {
    pub id: egui::Id,
    scroll_id: egui::Id,
    height: f32,
    row_extent: f32,
}
#[derive(Clone, Default)]
struct State {
    rect: Option<egui::Rect>,
    focus_requested: bool,
}
pub(super) fn pointer_sense(drag: bool) -> Sense {
    let sense = if drag {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    sense.difference(Sense::FOCUSABLE)
}
impl Keyboard {
    pub(super) fn new(
        ui: &egui::Ui,
        salt: impl std::hash::Hash + std::fmt::Debug,
        height: f32,
        row_height: f32,
    ) -> Self {
        let scroll_id = ui.make_persistent_id(egui::IdSalt::new(salt));
        Self {
            id: scroll_id.with("collection_keyboard"),
            scroll_id,
            height: height.min(ui.available_height()).max(row_height),
            row_extent: row_height + ui.spacing().item_spacing.y,
        }
    }
    pub(super) fn page_rows(&self, ui: &egui::Ui) -> usize {
        let rect = ui
            .ctx()
            .data(|d| d.get_temp::<State>(self.id))
            .and_then(|s| s.rect);
        ((rect.map_or(self.height, |r| r.height()) / self.row_extent).floor() as usize)
            .saturating_sub(1)
            .max(1)
    }
    pub(super) fn events(
        &self,
        ui: &egui::Ui,
        accepts: impl Fn(egui::Key, egui::Modifiers) -> bool,
    ) -> Vec<(egui::Key, egui::Modifiers, bool)> {
        let rect = ui
            .ctx()
            .data(|d| d.get_temp::<State>(self.id))
            .and_then(|s| s.rect);
        let outside = ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| rect.is_none_or(|r| !r.contains(p)))
        });
        if !ui.is_enabled()
            || !ui.memory(|m| m.has_focus(self.id))
            || egui::DragAndDrop::has_any_payload(ui.ctx())
            || egui::Popup::is_any_open(ui.ctx())
            || outside
        {
            return vec![];
        }
        let keys = ui.input_mut(|input| {
            let mut keys = Vec::new();
            input.events.retain(|event| {
                let egui::Event::Key {
                    key,
                    modifiers,
                    pressed,
                    repeat,
                    ..
                } = event
                else {
                    return true;
                };
                if !accepts(*key, *modifiers) {
                    return true;
                }
                if *pressed {
                    keys.push((*key, *modifiers, *repeat));
                }
                false
            });
            keys
        });
        if !keys.is_empty() {
            // Newly focused containers have no filter until the next pass.
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        keys
    }
    pub(super) fn scroll(
        &self,
        ui: &egui::Ui,
        scroll: egui::ScrollArea,
        row: usize,
    ) -> egui::ScrollArea {
        let offset =
            egui::scroll_area::State::load(ui.ctx(), self.scroll_id).map_or(0., |s| s.offset.y);
        let top = row as f32 * self.row_extent;
        if top < offset {
            scroll.vertical_scroll_offset(top)
        } else if top + self.row_extent > offset + self.height {
            scroll.vertical_scroll_offset((top + self.row_extent - self.height).max(0.))
        } else {
            scroll
        }
    }
    pub(super) fn focus_on_click(&self, response: &egui::Response) {
        if response.clicked() {
            response
                .ctx
                .data_mut(|d| d.get_temp_mut_or_default::<State>(self.id).focus_requested = true);
        }
    }
    pub(super) fn has_focus(&self, ui: &egui::Ui) -> bool {
        ui.memory(|m| m.has_focus(self.id))
    }
    pub(super) fn finish(&self, ui: &egui::Ui, rect: egui::Rect, label: &str) {
        ui.ctx()
            .data_mut(|d| d.get_temp_mut_or_default::<State>(self.id).rect = Some(rect));
        let response = ui.interact(rect, self.id, Sense::focusable_noninteractive());
        // Row hit testing can surrender focus during container registration.
        let request = ui.ctx().data_mut(|d| {
            std::mem::take(&mut d.get_temp_mut_or_default::<State>(self.id).focus_requested)
        });
        if request {
            ui.memory_mut(|m| m.request_focus(self.id));
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), label)
        });
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                self.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                2.,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
}
