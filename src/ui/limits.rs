//! Memory and cache limits, said plainly when they get in the way: a photo
//! too large to open at full size without asking, full-size detail memory
//! that cannot hold the photos being compared, and a disk cache that removes
//! previews made in this same session. Each prompt offers the way out and can
//! be turned off for good; none blocks the window.
use super::*;
use crate::core::{
    cache::{DISK_MAX_MIB, Limits, MIB},
    memory::{Fit, Machine, Pressure, size_text},
    preferences::LimitPrompt,
};

/// Memory for photo textures: 1/128 of the computer's memory, 96 MB to
/// 512 MB, and less while the system is short of memory.
pub(super) fn texture_budget() -> usize {
    let base = (Machine::current().ram / 128).clamp(96 * MIB, 512 * MIB) as usize;
    Pressure::current().share(base)
}

/// Session state. What the user turned off for good lives in Preferences.
#[derive(Default)]
pub(super) struct State {
    /// Photos the user chose to open at full size although they are large.
    pub open_large: HashSet<MediaId>,
    /// Photos whose large-photo card was answered "Not now".
    declined: HashSet<MediaId>,
    /// Prompts already shown in this session.
    asked: HashSet<LimitPrompt>,
    notice: Option<Notice>,
    /// The memory pressure last seen, to release textures once it rises.
    pressure: Pressure,
    /// The counters when the open folder was opened: prompts are about
    /// what happened in this folder, not in one opened before.
    folder: Baseline,
}
#[derive(Clone, Copy, Debug, Default)]
struct Baseline {
    epoch: u64,
    written: u64,
    photos: u64,
    evicted: u64,
    detail_overflow: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Notice {
    pub prompt: LimitPrompt,
    pub title: String,
    pub body: String,
    /// The setting it offers, in MiB.
    pub increase_to: u32,
    pub quiet: bool,
}

fn computer() -> &'static str {
    if cfg!(target_os = "macos") {
        "this Mac"
    } else {
        "this computer"
    }
}
/// A setting in MiB in the words Settings uses.
fn mib_text(mib: u32) -> String {
    size_text(u64::from(mib) * MIB)
}
/// The smallest power of two of MiB holding `bytes`, at least `floor` MiB.
fn holding(bytes: u64, floor: u32) -> u32 {
    let mib = bytes.div_ceil(MIB).max(1);
    (mib.next_power_of_two().min(u64::from(u32::MAX)) as u32).max(floor)
}

/// The detail memory prompt for an image of `bytes` too large for `limits`:
/// enough for two of them, so comparing or going back stays fast. None when
/// Settings cannot offer more.
pub(super) fn detail_notice(bytes: u64, limits: Limits, max_mib: u32) -> Option<Notice> {
    let increase_to = holding(bytes * 2, limits.originals_mib).min(max_mib);
    (increase_to > limits.originals_mib).then(|| Notice {
        prompt: LimitPrompt::DetailMemory,
        title: "Full-size photos don't fit in detail memory".into(),
        body: format!(
            "This photo takes {} at full size and detail memory is {}, so comparing photos or going back to one decodes it again, which is slow.",
            size_text(bytes),
            mib_text(limits.originals_mib)
        ),
        increase_to,
        quiet: false,
    })
}
/// The disk cache prompt once a trim removed previews of this session.
/// `needed` is what the folder would take; the offer leaves half the free
/// space. None when no larger limit fits.
pub(super) fn disk_notice(needed: u64, limits: Limits, free: Option<u64>) -> Option<Notice> {
    let room = free.map_or(DISK_MAX_MIB, |free| (free / MIB / 2) as u32);
    let wanted = (needed + needed / 4).div_ceil(1024 * MIB) as u32 * 1024;
    let increase_to = wanted
        .max(limits.disk_mib.saturating_mul(2))
        .min(room)
        .min(DISK_MAX_MIB);
    (limits.disk_mib > 0 && increase_to > limits.disk_mib).then(|| Notice {
        prompt: LimitPrompt::DiskCache,
        title: "The disk cache is full".into(),
        body: format!(
            "PhotoSort is removing previews it made earlier, so they are made again the next time you open this folder, which takes longer. This folder needs about {}; the limit is {}{}.",
            size_text(needed),
            mib_text(limits.disk_mib),
            free.map(|free| format!(" and {} is free", size_text(free)))
                .unwrap_or_default()
        ),
        increase_to,
        quiet: false,
    })
}

impl PhotoSortApp {
    /// A full-size decode was refused for memory. With "Always open large
    /// photos" on, a photo that still fits in memory opens without asking.
    pub(super) fn on_too_large(&mut self, id: MediaId, too_large: TooLarge) {
        if self.preferences.limit_prompts.open_large
            && Machine::current().fit(too_large.needed) == Fit::Large
        {
            self.limits.open_large.insert(id);
        }
    }

    /// Has the background builder make this folder's previews on disk, so
    /// photos show at once when browsed; a no-op for the same folder.
    pub(super) fn build_previews(&self) {
        // Outward from the photo on screen, which is wanted first.
        let start = self
            .active
            .and_then(|id| self.index.get(&id).copied())
            .unwrap_or(0);
        self.runtime.previews.build(
            Arc::new(self.media.clone()),
            start,
            self.preferences.cache,
            self.preferences.raw_look,
        );
    }

    /// Once per frame: whether the image worker's counters call for a prompt.
    pub(super) fn check_limits(&mut self) {
        let pressure = Pressure::current();
        if pressure != self.limits.pressure {
            self.limits.pressure = pressure;
            // Smaller under pressure: let go of textures not on screen now.
            self.evict(0);
        }
        // Every pipeline's disk cache writes: the image worker's and the builder's.
        let writes = self.runtime.cache.writes();
        let load = |n: &std::sync::atomic::AtomicU64| n.load(std::sync::atomic::Ordering::Relaxed);
        let now = Baseline {
            epoch: self.runtime.epoch(),
            written: load(&writes.written),
            photos: load(&writes.photos),
            evicted: load(&writes.evicted),
            detail_overflow: self.stats.detail_overflow,
        };
        if now.epoch != self.limits.folder.epoch {
            // Another folder: an unanswered prompt about the last one goes,
            // and may come back if this folder runs into the same limit.
            if let Some(notice) = self.limits.notice.take() {
                self.limits.asked.remove(&notice.prompt);
            }
            self.limits.folder = now;
        }
        if self.limits.notice.is_some() {
            return;
        }
        let folder = self.limits.folder;
        let quiet = &self.preferences.limit_prompts.quiet;
        let can_ask = |prompt| !quiet.contains(&prompt) && !self.limits.asked.contains(&prompt);
        let limits = self.preferences.cache;
        // A photo of this folder was too large for detail memory.
        let detail = (can_ask(LimitPrompt::DetailMemory)
            && now.detail_overflow != folder.detail_overflow
            && now.detail_overflow > u64::from(limits.originals_mib) * MIB)
            .then(|| detail_notice(now.detail_overflow, limits, Limits::originals_max_mib()))
            .flatten();
        let notice = detail.or_else(|| {
            // A trim removed previews made while this folder was open.
            let (written, photos) = match now.photos - folder.photos {
                0 => (now.written, now.photos),
                photos => (now.written - folder.written, photos),
            };
            (can_ask(LimitPrompt::DiskCache) && now.evicted > folder.evicted && photos > 0)
                .then(|| {
                    disk_notice(
                        written / photos * self.media.len().max(1) as u64,
                        limits,
                        Machine::current().free_disk,
                    )
                })
                .flatten()
        });
        if let Some(notice) = notice {
            self.limits.asked.insert(notice.prompt);
            self.limits.notice = Some(notice);
        }
    }

    /// Raises one limit now and remembers it, as Settings would.
    fn increase_limit(&mut self, prompt: LimitPrompt, mib: u32) {
        let mut cache = self.preferences.cache;
        match prompt {
            LimitPrompt::DetailMemory => cache.originals_mib = mib,
            LimitPrompt::DiskCache => cache.disk_mib = mib,
        }
        cache.normalize();
        self.preferences.cache = cache;
        self.runtime.cache.configure(cache);
        self.save_preferences();
        self.build_previews();
    }

    /// Ends the prompt: raises the limit if asked, and remembers "Don't ask again".
    pub(super) fn answer_limit_notice(&mut self, increase: bool) {
        let Some(notice) = self.limits.notice.take() else {
            return;
        };
        if notice.quiet {
            self.preferences.limit_prompts.quiet.insert(notice.prompt);
        }
        if increase {
            self.increase_limit(notice.prompt, notice.increase_to);
        } else if notice.quiet {
            self.save_preferences();
        }
    }

    /// The prompt above the status bar, until it is answered.
    pub(super) fn show_limit_notice(&mut self, ctx: &egui::Context) {
        let Some(notice) = &mut self.limits.notice else {
            return;
        };
        let mut answer = None;
        egui::Area::new(egui::Id::new("limit_notice"))
            .order(egui::Order::Foreground)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -(navigation::STATUS_HEIGHT + S4)),
            )
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(ELEVATED)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(R_SURFACE)
                    .inner_margin(S3)
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: Color32::from_black_alpha(90),
                    })
                    .show(ui, |ui| {
                        ui.set_max_width(440.0);
                        ui.spacing_mut().item_spacing.y = S1;
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(egui_phosphor::regular::GAUGE)
                                    .size(16.0)
                                    .color(MUTED),
                            );
                            ui.label(RichText::new(&notice.title).font(semibold(14.0)));
                        });
                        ui.add(
                            egui::Label::new(RichText::new(&notice.body).small().color(MUTED))
                                .wrap(),
                        );
                        ui.add_space(S1);
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut notice.quiet, "Don't ask again");
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .button(format!("Increase to {}", mib_text(notice.increase_to)))
                                    .clicked()
                                {
                                    answer = Some(true);
                                }
                                if ui.button("Keep as is").clicked() {
                                    answer = Some(false);
                                }
                            });
                        });
                    });
            });
        if let Some(increase) = answer {
            self.answer_limit_notice(increase);
        }
    }

    /// Over a photo whose full size was refused for memory: why, and how to
    /// open it anyway when the computer can. Nothing once answered "Not now".
    pub(super) fn large_photo_card(&mut self, ui: &mut egui::Ui, id: MediaId, too_large: TooLarge) {
        if self.limits.declined.contains(&id) {
            // Answered "Not now": a quiet way back for the rest of the session.
            let again = ui.add(
                egui::Label::new(RichText::new("Full size not opened").small().color(MUTED))
                    .sense(egui::Sense::click()),
            );
            if again
                .on_hover_text("Show why, and open it at full size")
                .clicked()
            {
                self.limits.declined.remove(&id);
            }
            return;
        }
        let machine = Machine::current();
        let fit = machine.fit(too_large.needed);
        let allowed = self.limits.open_large.contains(&id);
        let megapixels =
            (u64::from(too_large.width) * u64::from(too_large.height)).div_ceil(1_000_000);
        let needed = size_text(too_large.needed);
        let ram = size_text(machine.ram);
        let (title, body, action) = match fit {
            Fit::Fits | Fit::Large if !allowed => (
                format!("Full size needs about {needed} of memory"),
                format!(
                    "This {megapixels} MP photo is large for {} ({ram}). Opening it can take a while, and other apps may slow down meanwhile. The preview stays sharp up to fit size.",
                    computer()
                ),
                Some("Open anyway"),
            ),
            Fit::Exceeds if !allowed => (
                format!("This photo needs more memory than {} has", computer()),
                format!(
                    "Opening it at full size needs about {needed}; {} has {ram}. The system would have to swap, which is very slow.",
                    computer()
                ),
                Some("Try anyway"),
            ),
            _ => (
                format!("This photo needs more memory than {} has", computer()),
                format!(
                    "Opening it at full size needs about {needed}; {} has {ram}. The preview is shown instead.",
                    computer()
                ),
                None,
            ),
        };
        let mut always = self.preferences.limit_prompts.open_large;
        let mut open = false;
        let mut dismiss = false;
        egui::Frame::new()
            .fill(ELEVATED)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(R_SURFACE)
            .inner_margin(S3)
            .show(ui, |ui| {
                ui.set_max_width(380.0);
                ui.spacing_mut().item_spacing.y = S1;
                ui.label(RichText::new(title).font(semibold(14.0)));
                ui.add(egui::Label::new(RichText::new(body).small().color(MUTED)).wrap());
                ui.add_space(S1);
                ui.horizontal(|ui| {
                    if action == Some("Open anyway") {
                        ui.checkbox(&mut always, "Always open large photos");
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if let Some(action) = action {
                            open = ui.button(action).clicked();
                        }
                        dismiss = ui
                            .button(if action.is_some() { "Not now" } else { "OK" })
                            .clicked();
                    });
                });
            });
        if always != self.preferences.limit_prompts.open_large {
            self.preferences.limit_prompts.open_large = always;
            self.save_preferences();
        }
        if open {
            self.limits.open_large.insert(id);
        }
        if dismiss {
            self.limits.declined.insert(id);
        }
    }

    /// "Loading full size · 240 MP" once full-size detail takes a while, so
    /// a slow photo says why it is not sharp yet.
    pub(super) fn detail_loading_label(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        slot: u64,
        id: MediaId,
    ) {
        const AFTER: Duration = Duration::from_millis(500);
        let since = *self.detail_waiting.entry(slot).or_insert_with(Instant::now);
        let waited = since.elapsed();
        if waited < AFTER {
            ui.ctx().request_repaint_after(AFTER - waited);
            return;
        }
        let megapixels = self
            .metadata
            .get(&id)
            .map(|m| (u64::from(m.width) * u64::from(m.height)).div_ceil(1_000_000))
            .unwrap_or(0);
        let text = if megapixels > 0 {
            format!("Loading full size · {megapixels} MP")
        } else {
            "Loading full size".to_owned()
        };
        ui.painter().text(
            rect.left_bottom() + egui::vec2(10.0, -8.0),
            Align2::LEFT_BOTTOM,
            text,
            FontId::proportional(11.0),
            MUTED,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory::GIB;
    fn limits(originals_mib: u32, disk_mib: u32) -> Limits {
        Limits {
            previews_mib: 256,
            originals_mib,
            disk_mib,
        }
    }
    #[test]
    fn limit_prompts_offer_room_for_two_photos_or_the_whole_folder_within_bounds() {
        // A 150 MP RAW is 450 MB at full size: room for two is 1 GB.
        let notice = detail_notice(450 * MIB, limits(256, 4096), 8192).unwrap();
        assert_eq!(notice.increase_to, 1024);
        assert!(notice.body.contains("450 MB"), "{}", notice.body);
        assert!(notice.body.contains("256 MB"), "{}", notice.body);
        // Settings cannot offer more than half the memory.
        assert!(detail_notice(450 * MIB, limits(1024, 4096), 1024).is_none());
        // 18 GB needed: 23 GB offered, no more than half of 30 GB free.
        let disk = disk_notice(18 * GIB, limits(512, 4096), Some(200 * GIB)).unwrap();
        assert_eq!(disk.increase_to, 23 * 1024);
        assert!(disk.body.contains("200.0 GB is free"), "{}", disk.body);
        let tight = disk_notice(18 * GIB, limits(512, 4096), Some(10 * GIB)).unwrap();
        assert_eq!(tight.increase_to, 5 * 1024);
        assert!(disk_notice(18 * GIB, limits(512, 0), Some(200 * GIB)).is_none());
        assert!(disk_notice(18 * GIB, limits(512, 4096), Some(6 * GIB)).is_none());
    }
    #[test]
    fn a_limit_prompt_raises_its_setting_and_answers_stay_remembered() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.preferences.cache = limits(256, 4096);
        app.stats.detail_overflow = 450 * MIB;
        app.check_limits();
        assert_eq!(
            app.limits.notice.as_ref().map(|n| n.prompt),
            Some(LimitPrompt::DetailMemory)
        );
        // Unanswered when another folder opens: it goes, and asks again only
        // once a photo of the new folder is too large.
        app.runtime.next_epoch();
        app.check_limits();
        assert!(app.limits.notice.is_none());
        app.stats.detail_overflow = 500 * MIB;
        app.check_limits();
        assert_eq!(
            app.limits.notice.as_ref().map(|n| n.prompt),
            Some(LimitPrompt::DetailMemory)
        );
        app.limits.notice.as_mut().unwrap().quiet = true;
        app.answer_limit_notice(true);
        assert_eq!(app.preferences.cache.originals_mib, 1024);
        assert!(
            app.preferences
                .limit_prompts
                .quiet
                .contains(&LimitPrompt::DetailMemory)
        );
        // Answered: a larger photo later does not ask again.
        app.stats.detail_overflow = 3000 * MIB;
        app.check_limits();
        assert!(app.limits.notice.is_none());
        // A photo that still fits in memory opens without asking only once
        // "Always open large photos" is on.
        let id = MediaId::for_path(std::path::Path::new("/photos/large.IIQ"));
        let large = TooLarge {
            needed: Machine::current().ram / 10 * 6,
            width: 14_204,
            height: 10_652,
        };
        app.on_too_large(id, large);
        assert!(!app.limits.open_large.contains(&id));
        app.preferences.limit_prompts.open_large = true;
        app.on_too_large(id, large);
        assert!(app.limits.open_large.contains(&id));
        app.runtime.shutdown();
    }
}
