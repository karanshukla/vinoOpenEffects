//! Accessibility regression tests: every control in the window has a name a
//! screen reader can read, decorative images are hidden, and the custom CSS
//! keeps WCAG AA text contrast. The widget test needs a display; without one
//! it prints `skipping: ...`, unless `VINOOPENEFFECTS_REQUIRE_DISPLAY` is set
//! (CI sets it), in which case it fails.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::app::{APP_CSS, build_primary_menu};
use crate::config::GuiConfig;
use crate::dbus_client::GuiCommand;
use crate::pages::{background, center_stage, reactions, studio_light};
use crate::{preferences, preview};

const AA_TEXT: f64 = 4.5;

type Rgba = [f64; 4];

/// `#rrggbb` or `rgba(r, g, b, a)`.
fn parse_colour(value: &str) -> Rgba {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let channel = |i: usize| f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap());
        return [channel(0), channel(2), channel(4), 1.0];
    }
    let inner = value
        .strip_prefix("rgba(")
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or_else(|| panic!("unsupported colour {value:?}"));
    let parts: Vec<f64> = inner
        .split(',')
        .map(|p| p.trim().parse().unwrap())
        .collect();
    [parts[0], parts[1], parts[2], parts[3]]
}

/// `fg` alpha-composited over an opaque `bg`.
fn over(fg: Rgba, bg: Rgba) -> Rgba {
    let a = fg[3];
    [
        fg[0] * a + bg[0] * (1.0 - a),
        fg[1] * a + bg[1] * (1.0 - a),
        fg[2] * a + bg[2] * (1.0 - a),
        1.0,
    ]
}

fn luminance(c: Rgba) -> f64 {
    let lin = |v: f64| {
        let v = v / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Declarations of the first rule for exactly `selector` in `css`.
fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
    let open = format!("{selector} {{");
    let start = css
        .find(&open)
        .unwrap_or_else(|| panic!("no rule for {selector:?}"))
        + open.len();
    let end = start + css[start..].find('}').unwrap();
    &css[start..end]
}

fn prop<'a>(decls: &'a str, name: &str) -> &'a str {
    decls
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim())
        .unwrap_or_else(|| panic!("no {name} in {decls:?}"))
}

fn high_contrast_css() -> &'static str {
    let at = APP_CSS
        .find("@media (prefers-contrast: more)")
        .expect("APP_CSS has a high-contrast block");
    &APP_CSS[at..]
}

const PILL: &str = ".oe-pill, .oe-pill-button, .oe-pill-dropdown > button";
const WHITE: Rgba = [255.0, 255.0, 255.0, 1.0];
const BLACK: Rgba = [0.0, 0.0, 0.0, 1.0];

#[test]
fn contrast_ratio_matches_wcag_reference_values() {
    assert!((contrast(BLACK, WHITE) - 21.0).abs() < 1e-9);
    assert!((contrast(WHITE, WHITE) - 1.0).abs() < 1e-9);
    // #767676 is the lightest grey that passes AA on white.
    let grey = parse_colour("#767676");
    assert!(contrast(grey, WHITE) >= AA_TEXT);
    assert!(contrast(parse_colour("#777777"), WHITE) < AA_TEXT);
}

#[test]
fn preview_status_text_meets_aa() {
    let frame = parse_colour(prop(rule(APP_CSS, ".oe-preview-frame"), "background-color"));
    for css in [APP_CSS, high_contrast_css()] {
        let fg = parse_colour(prop(rule(css, ".oe-preview-status"), "color"));
        let ratio = contrast(fg, frame);
        assert!(ratio >= AA_TEXT, "preview status text {ratio:.2}:1");
    }
}

/// Pills float over live video, so their text must pass over any frame: the
/// translucent background is checked over pure white and pure black.
#[test]
fn pill_text_meets_aa_over_any_video() {
    let fg = parse_colour(prop(rule(APP_CSS, PILL), "color"));
    let backgrounds = [
        ("pill", prop(rule(APP_CSS, PILL), "background-color")),
        (
            "hover",
            prop(
                rule(
                    APP_CSS,
                    ".oe-pill-button:hover, .oe-pill-dropdown > button:hover",
                ),
                "background-color",
            ),
        ),
        (
            "comparing",
            prop(
                rule(APP_CSS, ".oe-pill-button.oe-comparing"),
                "background-color",
            ),
        ),
    ];
    for (state, bg) in backgrounds {
        for video in [WHITE, BLACK] {
            let ratio = contrast(fg, over(parse_colour(bg), video));
            assert!(ratio >= AA_TEXT, "{state} pill text {ratio:.2}:1");
        }
    }

    let hc = rule(high_contrast_css(), PILL);
    let fg = parse_colour(prop(hc, "color"));
    for video in [WHITE, BLACK] {
        let ratio = contrast(fg, over(parse_colour(prop(hc, "background-color")), video));
        assert!(ratio >= AA_TEXT, "high-contrast pill text {ratio:.2}:1");
    }
}

#[test]
fn font_sizes_follow_text_scaling() {
    // px font sizes ignore the desktop's text scaling factor.
    for decl in APP_CSS.split(';') {
        if let Some((key, value)) = decl.split_once(':')
            && key.trim().ends_with("font-size")
        {
            assert!(!value.contains("px"), "fixed font size: {decl}");
        }
    }
}

/// Initialise GTK + libadwaita with the test accessibility backend, or say
/// why not.
fn init_gtk() -> bool {
    let data = std::env::temp_dir().join("vinoopeneffects-a11y-test");
    // SAFETY: no other test reads these variables, and GTK isn't running yet.
    unsafe {
        std::env::set_var("GTK_A11Y", "test");
        std::env::set_var("XDG_DATA_HOME", &data);
    }
    match adw::init() {
        Ok(()) => true,
        Err(err) => {
            assert!(
                std::env::var_os("VINOOPENEFFECTS_REQUIRE_DISPLAY").is_none(),
                "GTK failed to initialise: {err}"
            );
            eprintln!("skipping: no display for GTK ({err})");
            false
        }
    }
}

fn has_name(widget: &gtk::Widget) -> bool {
    gtk::test_accessible_has_property(widget, gtk::AccessibleProperty::Label)
        || gtk::test_accessible_has_relation(widget, gtk::AccessibleRelation::LabelledBy)
        || (widget.is::<gtk::Button>() && has_text(widget))
}

/// A button is named by its content when it holds visible text.
fn has_text(widget: &gtk::Widget) -> bool {
    descendants(widget).iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|l| !l.label().trim().is_empty())
    })
}

fn is_hidden_from_at(widget: &gtk::Widget) -> bool {
    gtk::test_accessible_has_role(widget, gtk::AccessibleRole::Presentation)
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut child = widget.first_child();
    while let Some(c) = child {
        out.push(c.clone());
        out.extend(descendants(&c));
        child = c.next_sibling();
    }
    out
}

fn describe(widget: &gtk::Widget) -> String {
    format!(
        "{} (tooltip {:?}, classes {:?})",
        widget.type_().name(),
        widget.tooltip_text(),
        widget.css_classes()
    )
}

#[derive(Default)]
struct Audit {
    controls: usize,
    problems: Vec<String>,
}

impl Audit {
    fn check(&mut self, widget: &gtk::Widget) {
        // Banners (and their buttons) are libadwaita's; their title is the name.
        if widget.is::<adw::Banner>() {
            return;
        }
        // These rows expose their title as the control's name.
        if widget.is::<adw::SwitchRow>() || widget.is::<adw::ComboRow>() {
            self.controls += 1;
            if widget
                .downcast_ref::<adw::PreferencesRow>()
                .is_some_and(|r| r.title().is_empty())
            {
                self.problems
                    .push(format!("untitled row: {}", describe(widget)));
            }
            return;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.has_css_class("caption-heading")
            && !gtk::test_accessible_has_role(label, gtk::AccessibleRole::Heading)
        {
            self.problems.push(format!(
                "section label {:?} is not a heading",
                label.label()
            ));
        }
        if (widget.is::<gtk::Picture>()
            || (widget.is::<gtk::Image>()
                && widget.css_classes().iter().any(|c| c.starts_with("oe-"))))
            && !is_hidden_from_at(widget)
            && !gtk::test_accessible_has_property(widget, gtk::AccessibleProperty::Label)
        {
            self.problems.push(format!(
                "image neither named nor hidden: {}",
                describe(widget)
            ));
        }

        let control = widget.is::<gtk::Button>()
            || widget.is::<gtk::MenuButton>()
            || widget.is::<gtk::Switch>()
            || widget.is::<gtk::Scale>()
            || widget.is::<gtk::SpinButton>()
            || widget.is::<gtk::DropDown>()
            || widget.is::<adw::ToggleGroup>();
        if control {
            self.controls += 1;
            if widget.is_visible() && !has_name(widget) {
                self.problems
                    .push(format!("unnamed control: {}", describe(widget)));
            }
            // The name already says what the button is; its icon is noise.
            if widget.is::<gtk::Button>() {
                for image in descendants(widget) {
                    if (image.is::<gtk::Image>() || image.is::<gtk::Picture>())
                        && !is_hidden_from_at(&image)
                    {
                        self.problems.push(format!(
                            "decorative image not hidden in {}",
                            describe(widget)
                        ));
                    }
                }
            }
            // A control's internals (toggles, spin arrows, popovers) belong to
            // GTK/libadwaita, which name them from the control.
            return;
        }

        let mut child = widget.first_child();
        while let Some(c) = child {
            self.check(&c);
            child = c.next_sibling();
        }
    }
}

#[test]
fn every_control_has_an_accessible_name() {
    if !init_gtk() {
        return;
    }
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::unbounded_channel::<GuiCommand>();
    let mut audit = Audit::default();

    let center_stage = center_stage::build(&cmd_tx);
    let background = background::build(&cmd_tx);
    let studio_light = studio_light::build(&cmd_tx);
    let reactions = reactions::build(&cmd_tx);
    for card in [
        &center_stage.card,
        &background.card,
        &studio_light.card,
        &reactions.card,
    ] {
        audit.check(card.list.upcast_ref());
    }

    let preview = preview::build(&cmd_tx, Rc::new(RefCell::new(Vec::new())));
    audit.check(preview.frame.upcast_ref());

    audit.check(build_primary_menu().upcast_ref());

    let (dialog, _prefs) = preferences::build(&cmd_tx, Rc::new(RefCell::new(GuiConfig::default())));
    let pages: Vec<_> = descendants(dialog.upcast_ref())
        .into_iter()
        .filter(|w| w.is::<adw::PreferencesPage>())
        .collect();
    assert!(!pages.is_empty(), "no preferences page found");
    for page in &pages {
        audit.check(page);
    }

    // Guards against the walk silently finding nothing.
    assert!(
        audit.controls >= 40,
        "only {} controls found",
        audit.controls
    );
    assert!(
        audit.problems.is_empty(),
        "accessibility problems:\n{}",
        audit.problems.join("\n")
    );
}
