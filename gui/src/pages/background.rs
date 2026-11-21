//! Background card, driving two effects: `portrait_blur` and `bg_replace`.
//! Its Blur/Replace mode is not stored; `bg_replace.enabled` means Replace,
//! and choosing a mode enables one effect and disables the other.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk::RGBA;
use gtk::gio;
use shared::dbus::{VariantMap, value_as_bool, value_as_string, value_as_u32};

use crate::constants::{BG_BLUR, BG_IMAGE_PRESETS, BG_MODES, BG_PRESETS, BG_REPLACE, BLUR_LEVELS};
use crate::dbus_client::{CmdTx, GuiCommand};
use crate::widgets::{EffectCard, ToggleCtl, plain_row, section_label, toggle_row};

const CUSTOM_SWATCH_CLASS: &str = "oe-custom-swatch";
const CUSTOM_IMAGE_CLASS: &str = "oe-custom-image";

/// Size of the background-image preset thumbnails, sized so five fit across
/// the controls column.
const IMAGE_THUMB_WIDTH: i32 = 60;
const IMAGE_THUMB_HEIGHT: i32 = 40;

/// Widgets the state-sync layer keeps in sync with the daemon.
pub struct Widgets {
    pub card: EffectCard,
    mode_row: adw::ActionRow,
    mode: ToggleCtl,
    /// Blur-strength row, shown only in Blur mode.
    blur_row: adw::ActionRow,
    blur_strength: ToggleCtl,
    /// Rows shown only in Replace mode (image picker + color swatches).
    replace_rows: Vec<gtk::ListBoxRow>,
    /// Shows the file name of a custom (non-preset) image background.
    image_row: adw::ActionRow,
    /// Built-in image backgrounds: label, on-disk path and toggle button.
    image_presets: Vec<ImagePreset>,
    swatches: Vec<gtk::ToggleButton>,
    custom_provider: gtk::CssProvider,
    /// Display name of the current replacement background, for the summary.
    bg_name: RefCell<String>,
    /// Guards programmatic widget updates so they don't echo to the daemon.
    applying: Rc<Cell<bool>>,
    blur_enabled: Cell<bool>,
    bg_enabled: Cell<bool>,
}

pub fn build(cmd_tx: &CmdTx) -> Widgets {
    install_swatch_css();
    let applying = Rc::new(Cell::new(false));

    let (mode_row, mode) = build_mode_row(cmd_tx, &applying);
    let (blur_row, blur_strength) = build_blur_strength_row(cmd_tx, &applying);

    let card = build_card(cmd_tx, &applying, &mode.group);
    card.add_row(&mode_row);
    card.add_row(&blur_row);

    let mut group_leader: Option<gtk::ToggleButton> = None;

    let (images_row, image_presets) = build_images_row(cmd_tx, &applying, &mut group_leader);
    card.add_row(&images_row);

    let image_row = adw::ActionRow::builder()
        .title("No image selected")
        .subtitle("—")
        .visible(false)
        .build();
    card.add_row(&image_row);

    let (colors_row, swatches, custom_provider) =
        build_colors_row(cmd_tx, &applying, &mut group_leader);
    card.add_row(&colors_row);

    let widgets = Widgets {
        card,
        mode_row,
        mode,
        blur_row,
        blur_strength,
        replace_rows: vec![images_row, colors_row],
        image_row,
        image_presets,
        swatches,
        custom_provider,
        bg_name: RefCell::new(String::new()),
        applying,
        blur_enabled: Cell::new(false),
        bg_enabled: Cell::new(false),
    };
    widgets.update_visibility();
    widgets
}

fn build_mode_row(cmd_tx: &CmdTx, applying: &Rc<Cell<bool>>) -> (adw::ActionRow, ToggleCtl) {
    let cmd_tx = cmd_tx.clone();
    let applying = applying.clone();
    toggle_row("Mode", "", BG_MODES, move |value| {
        if applying.get() {
            return;
        }
        set_mode(&cmd_tx, value == BG_REPLACE);
    })
}

fn build_blur_strength_row(
    cmd_tx: &CmdTx,
    applying: &Rc<Cell<bool>>,
) -> (adw::ActionRow, ToggleCtl) {
    let cmd_tx = cmd_tx.clone();
    let applying = applying.clone();
    toggle_row("Strength", "", BLUR_LEVELS, move |value| {
        if applying.get() {
            return;
        }
        let strength: u32 = value.parse().unwrap_or(66);
        let _ = cmd_tx.send(GuiCommand::SetParam {
            id: "portrait_blur".into(),
            key: "strength".into(),
            value: shared::dbus::u32_value(strength),
        });
    })
}

fn build_card(
    cmd_tx: &CmdTx,
    applying: &Rc<Cell<bool>>,
    mode_group: &adw::ToggleGroup,
) -> EffectCard {
    let cmd_tx = cmd_tx.clone();
    let applying = applying.clone();
    let mode_toggle = mode_group.clone();
    EffectCard::new("Background", "view-conceal-symbolic", move |on| {
        if applying.get() {
            return;
        }
        if on {
            let replace = mode_toggle.active_name().as_deref() == Some(BG_REPLACE);
            set_mode(&cmd_tx, replace);
        } else {
            let _ = cmd_tx.send(GuiCommand::SetEnabled {
                id: "portrait_blur".into(),
                on: false,
            });
            let _ = cmd_tx.send(GuiCommand::SetEnabled {
                id: "bg_replace".into(),
                on: false,
            });
        }
    })
}

type ImagePreset = (&'static str, PathBuf, gtk::ToggleButton);

fn build_images_row(
    cmd_tx: &CmdTx,
    applying: &Rc<Cell<bool>>,
    group_leader: &mut Option<gtk::ToggleButton>,
) -> (gtk::ListBoxRow, Vec<ImagePreset>) {
    let image_box = wrapping_grid(5);

    let mut image_presets = Vec::new();
    for (label, path) in ensure_preset_images() {
        let button = image_preset_button(label, &path);
        match &*group_leader {
            Some(leader) => button.set_group(Some(leader)),
            None => *group_leader = Some(button.clone()),
        }
        {
            let cmd_tx = cmd_tx.clone();
            let applying = applying.clone();
            let path_str = path.to_string_lossy().into_owned();
            button.connect_toggled(move |b| {
                if applying.get() || !b.is_active() {
                    return;
                }
                set_background(&cmd_tx, &path_str);
            });
        }
        image_box.append(&button);
        image_presets.push((label, path, button));
    }

    let browse = gtk::Button::builder()
        .tooltip_text("Choose Image…")
        .css_classes(["oe-swatch-btn", "flat"])
        .build();
    browse.update_property(&[gtk::accessible::Property::Label("Choose image")]);
    let browse_child = image_box_child(CUSTOM_IMAGE_CLASS);
    browse_child.append(&plus_icon());
    browse.set_child(Some(&browse_child));
    {
        let cmd_tx = cmd_tx.clone();
        browse.connect_clicked(move |b| open_image_dialog(b, &cmd_tx));
    }
    image_box.append(&browse);
    let images_row = plain_row(&labelled(&section_label("Images"), &image_box));
    (images_row, image_presets)
}

fn build_colors_row(
    cmd_tx: &CmdTx,
    applying: &Rc<Cell<bool>>,
    group_leader: &mut Option<gtk::ToggleButton>,
) -> (gtk::ListBoxRow, Vec<gtk::ToggleButton>, gtk::CssProvider) {
    let swatch_box = wrapping_grid(7);

    let mut swatches = Vec::new();
    for (i, (label, hex)) in BG_PRESETS.iter().enumerate() {
        let button = swatch_button(&format!("oe-swatch-{i}"), label);
        match &*group_leader {
            Some(leader) => button.set_group(Some(leader)),
            None => *group_leader = Some(button.clone()),
        }
        {
            let cmd_tx = cmd_tx.clone();
            let applying = applying.clone();
            let hex = hex.to_string();
            button.connect_toggled(move |b| {
                if applying.get() || !b.is_active() {
                    return;
                }
                set_background(&cmd_tx, &hex);
            });
        }
        swatch_box.append(&button);
        swatches.push(button);
    }

    let custom_provider = gtk::CssProvider::new();
    add_provider(&custom_provider);
    let custom = gtk::Button::builder()
        .tooltip_text("Custom Color…")
        .css_classes(["oe-swatch-btn", "flat"])
        .build();
    custom.update_property(&[gtk::accessible::Property::Label("Custom color")]);
    let custom_child = swatch_box_child(CUSTOM_SWATCH_CLASS);
    custom_child.append(&plus_icon());
    custom.set_child(Some(&custom_child));
    {
        let cmd_tx = cmd_tx.clone();
        let provider = custom_provider.clone();
        custom.connect_clicked(move |b| open_color_dialog(b, &cmd_tx, &provider));
    }
    swatch_box.append(&custom);
    let colors_row = plain_row(&labelled(&section_label("Solid Colors"), &swatch_box));
    (colors_row, swatches, custom_provider)
}

impl Widgets {
    /// Apply an `EffectChanged`/`AllState` payload for `portrait_blur` or
    /// `bg_replace`.
    pub fn apply(&self, id: &str, params: &VariantMap) {
        self.applying.set(true);
        match id {
            "portrait_blur" => {
                if let Some(on) = params.get("enabled").and_then(value_as_bool) {
                    self.blur_enabled.set(on);
                }
                if let Some(strength) = params.get("strength").and_then(value_as_u32) {
                    self.blur_strength.set_value(&nearest_blur_level(strength));
                }
            }
            "bg_replace" => {
                if let Some(on) = params.get("enabled").and_then(value_as_bool) {
                    self.bg_enabled.set(on);
                }
                if let Some(bg) = params.get("background").and_then(value_as_string) {
                    self.apply_background(&bg);
                }
            }
            _ => {}
        }
        self.update_visibility();
        self.applying.set(false);
    }

    /// Replace if `bg_replace` is on, else Blur, also while the switch is off.
    fn current_mode(&self) -> &'static str {
        if self.bg_enabled.get() {
            return BG_REPLACE;
        }
        if self.blur_enabled.get() {
            return BG_BLUR;
        }
        match self.mode.group.active_name().as_deref() {
            Some(BG_REPLACE) => BG_REPLACE,
            _ => BG_BLUR,
        }
    }

    fn update_visibility(&self) {
        let on = self.blur_enabled.get() || self.bg_enabled.get();
        let mode = self.current_mode();

        self.card.set_on(on);
        self.mode.set_value(mode);
        self.mode_row.set_sensitive(on);
        self.blur_row.set_sensitive(on);
        self.blur_row.set_visible(mode == BG_BLUR);
        for row in &self.replace_rows {
            row.set_sensitive(on);
            row.set_visible(mode == BG_REPLACE);
        }
        self.image_row.set_sensitive(on);
        self.update_summary(on, mode);
    }

    fn update_summary(&self, on: bool, mode: &str) {
        let summary = if !on {
            "Off".to_string()
        } else if mode == BG_REPLACE {
            let name = self.bg_name.borrow();
            if name.is_empty() {
                "Replace".to_string()
            } else {
                format!("Replace · {name}")
            }
        } else {
            let strength = self.blur_strength.group.active_name();
            let label = BLUR_LEVELS
                .iter()
                .find(|(n, _)| Some(*n) == strength.as_deref())
                .map_or("Medium", |(_, label)| *label);
            format!("Blur · {label}")
        };
        self.card.set_summary(&summary);
    }

    /// Reflect the current `bg_replace.background` value in the swatch/image
    /// selection. Called with `applying` already set.
    fn apply_background(&self, bg: &str) {
        for button in &self.swatches {
            button.set_active(false);
        }
        for (_, _, button) in &self.image_presets {
            button.set_active(false);
        }
        let name = if bg.starts_with('#') {
            self.image_row.set_visible(false);
            if let Some(idx) = BG_PRESETS.iter().position(|(_, h)| *h == bg) {
                self.swatches[idx].set_active(true);
                BG_PRESETS[idx].0.to_string()
            } else if is_valid_hex_color(bg) {
                set_custom_color(&self.custom_provider, bg);
                "Custom color".to_string()
            } else {
                String::new()
            }
        } else if !bg.is_empty() {
            if let Some((label, _, button)) = self
                .image_presets
                .iter()
                .find(|(_, path, _)| path.to_str() == Some(bg))
            {
                button.set_active(true);
                self.image_row.set_visible(false);
                label.to_string()
            } else {
                let path = std::path::Path::new(bg);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| bg.to_string());
                self.image_row.set_title(&name);
                self.image_row.set_subtitle(bg);
                self.image_row.set_visible(true);
                name
            }
        } else {
            self.image_row.set_visible(false);
            String::new()
        };
        *self.bg_name.borrow_mut() = name;
    }
}

/// Enable the chosen mode's effect and disable the other one.
fn set_mode(cmd_tx: &CmdTx, replace: bool) {
    let _ = cmd_tx.send(GuiCommand::SetEnabled {
        id: "bg_replace".into(),
        on: replace,
    });
    let _ = cmd_tx.send(GuiCommand::SetEnabled {
        id: "portrait_blur".into(),
        on: !replace,
    });
}

/// Set the background value and switch to Replace mode.
fn set_background(cmd_tx: &CmdTx, value: &str) {
    let _ = cmd_tx.send(GuiCommand::SetParam {
        id: "bg_replace".into(),
        key: "background".into(),
        value: shared::dbus::str_value(value),
    });
    set_mode(cmd_tx, true);
}

fn open_image_dialog(widget: &impl IsA<gtk::Widget>, cmd_tx: &CmdTx) {
    let dialog = gtk::FileDialog::builder()
        .title("Choose Background Image")
        .build();
    let filter = gtk::FileFilter::new();
    filter.add_mime_type("image/png");
    filter.add_mime_type("image/jpeg");
    filter.set_name(Some("Images"));
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    let cmd_tx = cmd_tx.clone();
    let parent = widget.root().and_downcast::<gtk::Window>();
    dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res
            && let Some(path) = file.path()
        {
            set_background(&cmd_tx, &path.to_string_lossy());
        }
    });
}

fn open_color_dialog(button: &gtk::Button, cmd_tx: &CmdTx, provider: &gtk::CssProvider) {
    let dialog = gtk::ColorDialog::builder().title("Custom Color").build();
    let cmd_tx = cmd_tx.clone();
    let provider = provider.clone();
    let parent = button.root().and_downcast::<gtk::Window>();
    dialog.choose_rgba(parent.as_ref(), None, gio::Cancellable::NONE, move |res| {
        if let Ok(rgba) = res {
            let hex = rgba_to_hex(&rgba);
            set_custom_color(&provider, &hex);
            set_background(&cmd_tx, &hex);
        }
    });
}

/// Nearest blur-level toggle name for a raw strength value.
fn nearest_blur_level(strength: u32) -> String {
    BLUR_LEVELS
        .iter()
        .min_by_key(|(name, _)| {
            let level: i64 = name.parse().unwrap_or(66);
            (level - strength as i64).abs()
        })
        .map(|(name, _)| name.to_string())
        .unwrap_or_else(|| "66".into())
}

fn rgba_to_hex(rgba: &RGBA) -> String {
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        to_u8(rgba.red()),
        to_u8(rgba.green()),
        to_u8(rgba.blue())
    )
}

/// A grid that wraps its swatches/thumbnails onto extra rows when the column
/// is narrow, instead of forcing a minimum window width like a plain `Box`.
fn wrapping_grid(max_per_line: u32) -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(max_per_line)
        .row_spacing(8)
        .column_spacing(8)
        .halign(gtk::Align::Start)
        .build()
}

/// A section heading stacked over its content.
fn labelled(label: &gtk::Label, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    body.append(label);
    body.append(content);
    body
}

fn swatch_button(class: &str, tooltip: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .tooltip_text(tooltip)
        .css_classes(["oe-swatch-btn", "flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button.set_child(Some(&swatch_box_child(class)));
    button
}

fn swatch_box_child(class: &str) -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(32)
        .height_request(32)
        .css_classes(["oe-swatch", class])
        .build()
}

/// A "+" icon centered in its parent, for the custom-color and custom-image buttons.
fn plus_icon() -> gtk::Image {
    gtk::Image::builder()
        .icon_name("list-add-symbolic")
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build()
}

/// A toggle button showing a built-in background-image thumbnail.
fn image_preset_button(label: &str, path: &std::path::Path) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .tooltip_text(label)
        .css_classes(["oe-swatch-btn", "flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    let picture = gtk::Picture::builder()
        .file(&gio::File::for_path(path))
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(IMAGE_THUMB_WIDTH)
        .height_request(IMAGE_THUMB_HEIGHT)
        .css_classes(["oe-bg-thumb"])
        .overflow(gtk::Overflow::Hidden)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    button.set_child(Some(&picture));
    button
}

/// Thumbnail-sized box for the "add your own image" button.
fn image_box_child(class: &str) -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(IMAGE_THUMB_WIDTH)
        .height_request(IMAGE_THUMB_HEIGHT)
        .css_classes(["oe-bg-thumb", class])
        .build()
}

/// Write the bundled preset background images to the user's data dir (if not
/// already present) and return their paths: the daemon loads backgrounds from disk.
fn ensure_preset_images() -> Vec<(&'static str, PathBuf)> {
    let Some(base_dir) = dirs::data_dir().or_else(dirs::runtime_dir) else {
        return Vec::new();
    };
    let dir = base_dir.join("vinoopeneffects/backgrounds");
    let _ = std::fs::create_dir_all(&dir);
    BG_IMAGE_PRESETS
        .iter()
        .map(|preset| {
            let path = dir.join(preset.file);
            if !path.exists() {
                let _ = std::fs::write(&path, preset.bytes);
            }
            (preset.label, path)
        })
        .collect()
}

fn is_valid_hex_color(hex: &str) -> bool {
    hex.strip_prefix('#')
        .is_some_and(|rest| rest.len() == 6 && rest.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn set_custom_color(provider: &gtk::CssProvider, hex: &str) {
    if !is_valid_hex_color(hex) {
        return;
    }
    provider.load_from_string(&format!(
        ".{CUSTOM_SWATCH_CLASS} {{ background-color: {hex}; }}"
    ));
}

/// Install the static swatch CSS (base shape + one class per preset color).
/// Every preset is dark, so on a dark card only the border separates it.
pub(crate) const SWATCH_CSS: &str = ".oe-swatch { border-radius: 50%; border: 1px solid alpha(currentColor, 0.2); } \
     .oe-swatch-btn { padding: 4px; min-width: 0; min-height: 0; } \
     .oe-bg-thumb { border-radius: 10px; border: 1px solid alpha(currentColor, 0.2); } \
     .oe-custom-image { background-color: alpha(currentColor, 0.08); } \
     @media (prefers-color-scheme: dark) { \
       .oe-swatch { border-color: alpha(currentColor, 0.6); } } \
     @media (prefers-contrast: more) { \
       .oe-swatch, .oe-bg-thumb { border-color: alpha(currentColor, 0.6); } }";

fn install_swatch_css() {
    let mut css = String::from(SWATCH_CSS);
    for (i, (_, hex)) in BG_PRESETS.iter().enumerate() {
        css.push_str(&format!(" .oe-swatch-{i} {{ background-color: {hex}; }}"));
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    add_provider(&provider);
}

fn add_provider(provider: &gtk::CssProvider) {
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::gtk_test::{channel_pair, children, drain, run};

    fn applying() -> Rc<Cell<bool>> {
        Rc::new(Cell::new(false))
    }

    fn enabled(cmd: &GuiCommand) -> (&str, bool) {
        match cmd {
            GuiCommand::SetEnabled { id, on } => (id.as_str(), *on),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn the_mode_row_offers_blur_and_replace_and_sends_both_effect_flags() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let (row, mode) = build_mode_row(&tx, &applying());
            assert_eq!(row.title(), "Mode");
            assert_eq!(mode.group.n_toggles(), 2);
            mode.group.set_active_name(Some(BG_REPLACE));
            let sent = drain(&mut rx);
            assert_eq!(sent.len(), 2);
            assert_eq!(enabled(&sent[0]), ("bg_replace", true));
            assert_eq!(enabled(&sent[1]), ("portrait_blur", false));
        });
    }

    #[test]
    fn the_mode_row_stays_quiet_while_the_daemon_state_is_applied() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let guard = applying();
            let (_row, mode) = build_mode_row(&tx, &guard);
            guard.set(true);
            mode.group.set_active_name(Some(BG_REPLACE));
            assert!(drain(&mut rx).is_empty());
        });
    }

    #[test]
    fn the_blur_strength_row_offers_three_levels_and_sends_the_number() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let (row, strength) = build_blur_strength_row(&tx, &applying());
            assert_eq!(row.title(), "Strength");
            assert_eq!(strength.group.n_toggles(), BLUR_LEVELS.len() as u32);
            strength.group.set_active_name(Some("100"));
            let sent = drain(&mut rx);
            match sent.as_slice() {
                [GuiCommand::SetParam { id, key, value }] => {
                    assert_eq!((id.as_str(), key.as_str()), ("portrait_blur", "strength"));
                    assert_eq!(shared::dbus::value_as_u32(value), Some(100));
                }
                other => panic!("unexpected {other:?}"),
            }
        });
    }

    #[test]
    fn switching_the_card_on_picks_the_mode_and_off_disables_both_effects() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let guard = applying();
            let (_row, mode) = build_mode_row(&tx, &guard);
            let card = build_card(&tx, &guard, &mode.group);
            drain(&mut rx);
            let switch = card_switch(card.list.upcast_ref()).expect("card has a switch");
            switch.set_active(true);
            let on = drain(&mut rx);
            assert_eq!(enabled(&on[0]), ("bg_replace", false));
            assert_eq!(enabled(&on[1]), ("portrait_blur", true));
            switch.set_active(false);
            let off = drain(&mut rx);
            assert_eq!(enabled(&off[0]), ("portrait_blur", false));
            assert_eq!(enabled(&off[1]), ("bg_replace", false));
        });
    }

    fn card_switch(widget: &gtk::Widget) -> Option<gtk::Switch> {
        if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
            return Some(switch.clone());
        }
        children(widget).iter().find_map(card_switch)
    }

    #[test]
    fn the_colors_row_has_one_swatch_per_preset_in_one_radio_group() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let mut leader = None;
            let (row, swatches, _provider) = build_colors_row(&tx, &applying(), &mut leader);
            assert_eq!(swatches.len(), BG_PRESETS.len());
            for (button, (label, _)) in swatches.iter().zip(BG_PRESETS) {
                assert_eq!(button.tooltip_text().as_deref(), Some(*label));
            }
            assert!(row.child().is_some());
            swatches[1].set_active(true);
            assert!(swatches[1].is_active());
            swatches[2].set_active(true);
            assert!(!swatches[1].is_active(), "swatches are one radio group");
            let sent = drain(&mut rx);
            assert!(sent.iter().any(|c| matches!(c,
                GuiCommand::SetParam { key, value, .. }
                    if key == "background"
                        && shared::dbus::value_as_string(value).as_deref() == Some(BG_PRESETS[2].1))));
        });
    }

    #[test]
    fn the_images_row_has_one_thumbnail_per_preset_and_shares_the_color_radio_group() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let mut leader = None;
            let (_row, presets) = build_images_row(&tx, &applying(), &mut leader);
            assert_eq!(presets.len(), BG_IMAGE_PRESETS.len());
            for ((label, path, button), want) in presets.iter().zip(BG_IMAGE_PRESETS) {
                assert_eq!(*label, want.label);
                assert!(path.exists(), "{path:?} was not written");
                assert_eq!(button.tooltip_text().as_deref(), Some(*label));
            }
            let (_colors, swatches, _p) = build_colors_row(&tx, &applying(), &mut leader);
            presets[0].2.set_active(true);
            swatches[0].set_active(true);
            assert!(!presets[0].2.is_active(), "one background at a time");
            drain(&mut rx);
        });
    }

    #[test]
    fn the_whole_card_starts_in_blur_mode_with_the_replace_rows_hidden() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let widgets = build(&tx);
            assert!(widgets.replace_rows.iter().all(|r| !r.is_visible()));
            assert!(!widgets.image_row.is_visible());
            assert_eq!(widgets.image_row.title(), "No image selected");
            assert_eq!(widgets.current_mode(), BG_BLUR);
            assert_eq!(widgets.swatches.len(), BG_PRESETS.len());
            assert_eq!(widgets.image_presets.len(), BG_IMAGE_PRESETS.len());
        });
    }

    #[test]
    fn the_nearest_blur_level_rounds_to_a_toggle_name() {
        assert_eq!(nearest_blur_level(0), "33");
        assert_eq!(nearest_blur_level(50), "66");
        assert_eq!(nearest_blur_level(66), "66");
        assert_eq!(nearest_blur_level(90), "100");
        assert_eq!(nearest_blur_level(500), "100");
    }

    #[test]
    fn a_colour_is_written_as_lowercase_hex() {
        run(|| {
            assert_eq!(rgba_to_hex(&RGBA::new(1.0, 0.0, 0.5, 1.0)), "#ff0080");
            assert_eq!(rgba_to_hex(&RGBA::new(2.0, -1.0, 0.0, 1.0)), "#ff0000");
        });
    }

    #[test]
    fn only_rrggbb_reaches_the_swatch_css() {
        assert!(is_valid_hex_color("#ff0080"));
        assert!(is_valid_hex_color("#FF0080"));
        assert!(!is_valid_hex_color("ff0080"));
        assert!(!is_valid_hex_color("#ff00"));
        assert!(!is_valid_hex_color("#fff; } * { color: red"));
    }
}
