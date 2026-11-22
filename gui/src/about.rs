//! Native `AdwAboutDialog`, opened from the primary menu.

use adw::prelude::*;

use crate::constants::APP_ID;

/// The family's site and the other vinoPilot apps with their pages there: a
/// copy of vino-ui's `family` (karanshukla/vinoUserInterface, src/family.rs),
/// which the other Rust apps use directly. Not a dependency here because the
/// crate needs Rust 1.95 and this workspace builds on 1.92; keep it in step.
const SITE: (&str, &str) = ("vinopilot.ca", "https://vinopilot.ca/");
const OTHER_APPS: [(&str, &str, &str); 7] = [
    (
        "vinoPilot",
        "Install and update vinoPilot apps",
        "https://vinopilot.ca/apps/#vinopilot",
    ),
    (
        "vinoWhisper",
        "Live captions and voice typing",
        "https://vinopilot.ca/apps/#vinowhisper",
    ),
    (
        "vinoAuthFace",
        "Unlock with your face",
        "https://vinopilot.ca/apps/#vinoauthface",
    ),
    (
        "vinoPop",
        "AI actions for selected text",
        "https://vinopilot.ca/apps/#vinopop",
    ),
    (
        "vinoSpeak",
        "Natural-sounding text-to-speech",
        "https://vinopilot.ca/apps/#vinospeak",
    ),
    (
        "vinoSearch",
        "Search local files by meaning",
        "https://vinopilot.ca/apps/#vinosearch",
    ),
    (
        "vinoWatch",
        "Monitor NPU usage and activity",
        "https://vinopilot.ca/apps/#vinowatch",
    ),
];

pub fn present(parent: &impl IsA<gtk::Widget>) {
    let about = adw::AboutDialog::builder()
        .application_name("vinoOpenEffects")
        .application_icon(APP_ID)
        .version(shared::version::VERSION)
        .comments("Real-time webcam effects for Linux")
        .developer_name("Karan Shukla")
        .developers(["Karan Shukla https://github.com/karanshukla"])
        .copyright("© 2026 Karan Shukla\nBased on openeffects © Aryan Kushwaha and contributors")
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/karanshukla/vinoOpenEffects")
        .issue_url("https://github.com/karanshukla/vinoOpenEffects/issues")
        .build();

    about.add_credit_section(
        Some("Original Author of openeffects"),
        &["Aryan Kushwaha https://github.com/funinkina"],
    );
    about.add_link(
        "openeffects (upstream)",
        "https://github.com/funinkina/openeffects",
    );
    // The family, as every vinoPilot app lists it: the site, then the
    // other apps (under Details).
    about.add_link(SITE.0, SITE.1);
    for (name, blurb, page) in OTHER_APPS {
        about.add_link(&format!("{name}: {blurb}"), page);
    }

    about.present(Some(parent));
}
