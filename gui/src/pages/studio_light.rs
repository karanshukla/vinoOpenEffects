//! Studio Light card: the effect switch in the header, then an "On you"
//! section (intensity, brightness, contrast) and a "Behind you" section
//! (background brightness). The lift is applied to the person only
//! (foreground) and the background brightness to the complementary area
//! (inverted mask). An "Auto light" switch and strength slider drive the
//! NPU-only low-light model, insensitive without an NPU. Turning the switch
//! off disables the sliders.

use std::cell::Cell;

use adw::prelude::*;
use shared::dbus::{VariantMap, bool_value, value_as_bool, value_as_i32, value_as_u32};

use crate::dbus_client::{CmdTx, GuiCommand};
use crate::widgets::{
    EffectCard, SliderCtl, plain_row, section_label, slider_row_i32, slider_row_u32,
};

/// Widgets the state-sync layer keeps in sync with the daemon.
pub struct Widgets {
    pub card: EffectCard,
    intensity: SliderCtl,
    brightness: SliderCtl,
    contrast: SliderCtl,
    bg_brightness: SliderCtl,
    auto_light: adw::SwitchRow,
    auto_light_handler: gtk::glib::SignalHandlerId,
    auto_strength: SliderCtl,
    auto_strength_row: gtk::ListBoxRow,
    /// Dimmed while Studio Light is off.
    rows: Vec<gtk::ListBoxRow>,
    on: Cell<bool>,
    npu: Cell<bool>,
}

const AUTO_LIGHT_SUBTITLE: &str = "A low-light model that brightens a dim room";
const AUTO_LIGHT_NO_NPU: &str = "Needs an Intel NPU";

pub fn build(cmd_tx: &CmdTx) -> Widgets {
    let card = {
        let cmd_tx = cmd_tx.clone();
        EffectCard::new("Studio Light", "display-brightness-symbolic", move |on| {
            let _ = cmd_tx.send(GuiCommand::SetEnabled {
                id: "studio_light".into(),
                on,
            });
        })
    };

    let on_you = plain_row(&section_label("On You"));
    let (intensity_row, intensity) = slider_row_u32(
        "Intensity",
        "",
        0.0,
        100.0,
        1.0,
        "studio_light",
        "intensity",
        cmd_tx,
    );
    let (brightness_row, brightness) = slider_row_i32(
        "Brightness",
        "",
        -100.0,
        100.0,
        1.0,
        "studio_light",
        "brightness",
        cmd_tx,
    );
    let (contrast_row, contrast) = slider_row_u32(
        "Contrast",
        "",
        0.0,
        100.0,
        1.0,
        "studio_light",
        "contrast",
        cmd_tx,
    );
    let auto_section = plain_row(&section_label("Auto Light"));
    let auto_light = adw::SwitchRow::builder()
        .title("Brighten Dark Frames")
        .subtitle(AUTO_LIGHT_NO_NPU)
        .build();
    let auto_light_handler = {
        let cmd_tx = cmd_tx.clone();
        auto_light.connect_active_notify(move |row| {
            let _ = cmd_tx.send(GuiCommand::SetParam {
                id: "studio_light".into(),
                key: "auto_light".into(),
                value: bool_value(row.is_active()),
            });
        })
    };
    let (auto_strength_row, auto_strength) = slider_row_u32(
        "Strength",
        "",
        0.0,
        100.0,
        1.0,
        "studio_light",
        "auto_strength",
        cmd_tx,
    );
    let behind_you = plain_row(&section_label("Behind You"));
    let (bg_row, bg_brightness) = slider_row_i32(
        "Brightness",
        "Darken or brighten the room so you stand out",
        -100.0,
        100.0,
        1.0,
        "studio_light",
        "bg_brightness",
        cmd_tx,
    );

    auto_strength.set_accessible_name("Auto light strength");
    bg_brightness.set_accessible_name("Background brightness");

    let rows = vec![
        on_you,
        intensity_row,
        brightness_row,
        contrast_row,
        auto_section,
        auto_light.clone().upcast::<gtk::ListBoxRow>(),
        auto_strength_row.clone(),
        behind_you,
        bg_row,
    ];
    for row in &rows {
        card.add_row(row);
    }

    Widgets {
        card,
        intensity,
        brightness,
        contrast,
        bg_brightness,
        auto_light,
        auto_light_handler,
        auto_strength,
        auto_strength_row,
        rows,
        on: Cell::new(false),
        npu: Cell::new(false),
    }
}

impl Widgets {
    /// Auto light runs only on an NPU, so without one its rows stay insensitive.
    pub fn set_npu_available(&self, npu: bool) {
        self.npu.set(npu);
        self.auto_light.set_subtitle(if npu {
            AUTO_LIGHT_SUBTITLE
        } else {
            AUTO_LIGHT_NO_NPU
        });
        self.update_sensitivity();
    }

    #[cfg(test)]
    pub fn auto_light_sensitive(&self) -> bool {
        self.auto_light.is_sensitive() && self.auto_strength_row.is_sensitive()
    }

    fn update_sensitivity(&self) {
        let on = self.on.get();
        for row in &self.rows {
            row.set_sensitive(on);
        }
        let auto = on && self.npu.get();
        self.auto_light.set_sensitive(auto);
        self.auto_strength_row.set_sensitive(auto);
    }

    pub fn apply(&self, params: &VariantMap) {
        if let Some(on) = params.get("enabled").and_then(value_as_bool) {
            self.card.set_on(on);
            self.card.set_summary(if on {
                "Lifts you, settles the room"
            } else {
                "Off"
            });
            self.on.set(on);
            self.update_sensitivity();
        }
        if let Some(v) = params.get("intensity").and_then(value_as_u32) {
            self.intensity.set_value(v as f64);
        }
        if let Some(v) = params.get("brightness").and_then(value_as_i32) {
            self.brightness.set_value(v as f64);
        }
        if let Some(v) = params.get("contrast").and_then(value_as_u32) {
            self.contrast.set_value(v as f64);
        }
        if let Some(v) = params.get("bg_brightness").and_then(value_as_i32) {
            self.bg_brightness.set_value(v as f64);
        }
        if let Some(on) = params.get("auto_light").and_then(value_as_bool)
            && self.auto_light.is_active() != on
        {
            self.auto_light.block_signal(&self.auto_light_handler);
            self.auto_light.set_active(on);
            self.auto_light.unblock_signal(&self.auto_light_handler);
        }
        if let Some(v) = params.get("auto_strength").and_then(value_as_u32) {
            self.auto_strength.set_value(v as f64);
        }
    }
}
