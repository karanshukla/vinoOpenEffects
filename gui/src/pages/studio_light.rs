//! Studio Light card: "On you" and "Behind you" sliders, and Auto light,
//! which stays insensitive without an NPU.

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

fn build_card(cmd_tx: &CmdTx) -> EffectCard {
    let cmd_tx = cmd_tx.clone();
    EffectCard::new("Studio Light", "display-brightness-symbolic", move |on| {
        let _ = cmd_tx.send(GuiCommand::SetEnabled {
            id: "studio_light".into(),
            on,
        });
    })
}

fn build_auto_light_switch(cmd_tx: &CmdTx) -> (adw::SwitchRow, gtk::glib::SignalHandlerId) {
    let auto_light = adw::SwitchRow::builder()
        .title("Brighten Dark Frames")
        .subtitle(AUTO_LIGHT_NO_NPU)
        .build();
    let handler = {
        let cmd_tx = cmd_tx.clone();
        auto_light.connect_active_notify(move |row| {
            let _ = cmd_tx.send(GuiCommand::SetParam {
                id: "studio_light".into(),
                key: "auto_light".into(),
                value: bool_value(row.is_active()),
            });
        })
    };
    (auto_light, handler)
}

pub fn build(cmd_tx: &CmdTx) -> Widgets {
    let card = build_card(cmd_tx);

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
    let (auto_light, auto_light_handler) = build_auto_light_switch(cmd_tx);
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

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::gtk_test::{channel_pair, children, drain, pump, run};

    #[test]
    fn the_card_is_named_studio_light_and_starts_off() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let card = build_card(&tx);
            assert!(!card.is_on());
            let expander = card.list.first_child().unwrap();
            let expander = expander.downcast_ref::<adw::ExpanderRow>().unwrap();
            assert_eq!(expander.title(), "Studio Light");
            assert_eq!(expander.subtitle(), "Off");
        });
    }

    #[test]
    fn auto_light_is_sensitive_only_with_an_npu() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let widgets = build(&tx);
            let enabled: shared::dbus::VariantMap =
                [("enabled".to_string(), shared::dbus::bool_value(true))].into();
            widgets.apply(&enabled);
            for npu in [false, true] {
                widgets.set_npu_available(npu);
                assert_eq!(widgets.auto_light_sensitive(), npu, "npu {npu}");
            }
        });
    }

    #[test]
    fn the_card_switch_sends_studio_light_enabled() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let widgets = build(&tx);
            widgets.card.set_on(true);
            assert!(drain(&mut rx).is_empty(), "set_on must not echo");
            let switch = find_switch(widgets.card.list.upcast_ref()).expect("card has a switch");
            switch.set_active(false);
            let sent = drain(&mut rx);
            assert!(
                matches!(sent.as_slice(), [GuiCommand::SetEnabled { id, on: false }] if id == "studio_light"),
                "{sent:?}"
            );
        });
    }

    fn find_switch(widget: &gtk::Widget) -> Option<gtk::Switch> {
        if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
            return Some(switch.clone());
        }
        children(widget).iter().find_map(find_switch)
    }

    #[test]
    fn the_auto_light_switch_says_it_needs_an_npu_and_sends_its_value() {
        run(|| {
            let (tx, mut rx) = channel_pair();
            let (row, _handler) = build_auto_light_switch(&tx);
            assert_eq!(row.title(), "Brighten Dark Frames");
            assert_eq!(row.subtitle().as_deref(), Some("Needs an Intel NPU"));
            row.set_active(true);
            let sent = drain(&mut rx);
            match sent.as_slice() {
                [GuiCommand::SetParam { id, key, value }] => {
                    assert_eq!((id.as_str(), key.as_str()), ("studio_light", "auto_light"));
                    assert_eq!(shared::dbus::value_as_bool(value), Some(true));
                }
                other => panic!("unexpected {other:?}"),
            }
        });
    }

    #[test]
    fn the_rows_run_on_you_then_auto_light_then_behind_you() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let widgets = build(&tx);
            assert_eq!(widgets.rows.len(), 9);
            let headings: Vec<String> = widgets
                .rows
                .iter()
                .filter_map(|r| r.child())
                .filter_map(|c| c.downcast::<gtk::Label>().ok())
                .map(|l| l.label().to_string())
                .collect();
            assert_eq!(headings, ["On You", "Auto Light", "Behind You"]);
            assert!(widgets.rows[5].is::<adw::SwitchRow>());
            assert_eq!(widgets.rows[6], widgets.auto_strength_row);
        });
    }

    #[test]
    fn turning_studio_light_on_wakes_the_rows_but_auto_light_waits_for_an_npu() {
        run(|| {
            let (tx, _rx) = channel_pair();
            let widgets = build(&tx);
            let on: shared::dbus::VariantMap =
                [("enabled".to_string(), shared::dbus::bool_value(true))].into();
            widgets.apply(&on);
            assert!(widgets.rows[1].is_sensitive());
            assert!(!widgets.auto_light_sensitive());
            widgets.set_npu_available(true);
            assert!(widgets.auto_light_sensitive());
            pump(10);
        });
    }
}
