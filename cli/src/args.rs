use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "vinoopeneffects-ctl",
    about = "Control the vinoOpenEffects daemon",
    long_about = "Control the vinoOpenEffects daemon over D-Bus.\n\n\
        Effects have friendly subcommands (center-stage, portrait-blur, background,\n\
        studio-light, reactions). Run one with no flags to print its current state,\n\
        or pass flags to change settings. `set` is a low-level escape hatch.",
    version = shared::version::VERSION
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show daemon status and capabilities.
    Status {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Print only the one-word status (Waybar-friendly).
        #[arg(long)]
        short: bool,
    },
    /// Inspect system environment, models, NPU, cameras, and daemon health.
    Doctor {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Arm the virtual camera (camera opens on demand when a consumer connects).
    Start,
    /// Tear down the pipeline and release the camera.
    Stop,
    /// View or switch inference execution provider (auto, cpu, npu).
    Inference {
        /// Target provider (auto, cpu, npu). If omitted, displays current provider info.
        provider: Option<String>,
        /// Forget recorded NPU failures and reload the models.
        #[arg(long, conflicts_with = "provider")]
        retry: bool,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },

    /// List every effect with its current enabled state.
    Effects,

    /// Enable an effect.
    Enable {
        #[arg(value_enum)]
        effect: EffectArg,
    },
    /// Disable an effect.
    Disable {
        #[arg(value_enum)]
        effect: EffectArg,
    },
    /// Flip an effect on/off.
    Toggle {
        #[arg(value_enum)]
        effect: EffectArg,
    },

    /// Center Stage: auto crop+zoom that keeps you framed.
    CenterStage(CenterStageArgs),

    /// Portrait Blur: blur the background behind you.
    PortraitBlur {
        #[command(flatten)]
        toggle: EnableFlags,
        /// Blur strength (0-100).
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
        strength: Option<u8>,
    },

    /// Background replace: swap the background for a color or image.
    Background {
        /// `none` to clear/disable, a `#RRGGBB` color, or a path to an image.
        /// Omit to print the current background.
        value: Option<String>,
        /// Disable the effect without changing the stored background.
        #[arg(long)]
        off: bool,
    },

    /// Studio Light: brighten and add contrast to your face.
    StudioLight(StudioLightArgs),

    /// Reactions: gesture/emoji overlay. Toggle the effect or fire a one-shot burst.
    Reactions {
        #[command(flatten)]
        toggle: EnableFlags,
    },
    /// Fire a one-shot emoji burst.
    React {
        #[arg(value_enum)]
        reaction: Reaction,
    },

    Passthrough {
        /// Bypass all effects — raw camera passthrough.
        #[arg(long)]
        on: bool,
        /// Re-enable effects (turn passthrough off).
        #[arg(long, conflicts_with = "on")]
        off: bool,
    },

    /// Low-level: set any effect param directly (EFFECT.KEY VALUE).
    Set {
        /// e.g. studio_light.brightness
        assignment: String,
        value: String,
    },

    /// Camera selection and virtual-camera info.
    Camera {
        #[command(subcommand)]
        command: CameraCommand,
    },

    /// Manage starting the daemon automatically with the desktop session.
    Autostart {
        #[command(subcommand)]
        command: AutostartCommand,
    },

    /// Stream EffectChanged signals until interrupted.
    Watch,
}

#[derive(Debug, clap::Args)]
pub(crate) struct CenterStageArgs {
    #[command(flatten)]
    pub(crate) toggle: EnableFlags,
    /// Zoom level.
    #[arg(long, value_enum)]
    pub(crate) zoom: Option<Zoom>,
    /// Framing mode.
    #[arg(long, value_enum)]
    pub(crate) mode: Option<Mode>,
    /// Manual crop inset from the top edge (0-512 px).
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=512))]
    pub(crate) crop_top: Option<u32>,
    /// Manual crop inset from the bottom edge (0-512 px).
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=512))]
    pub(crate) crop_bottom: Option<u32>,
    /// Manual crop inset from the left edge (0-512 px).
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=512))]
    pub(crate) crop_left: Option<u32>,
    /// Manual crop inset from the right edge (0-512 px).
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=512))]
    pub(crate) crop_right: Option<u32>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct StudioLightArgs {
    #[command(flatten)]
    pub(crate) toggle: EnableFlags,
    /// Overall intensity (0-100).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub(crate) intensity: Option<u8>,
    /// Brightness lift (-100 to 100).
    #[arg(long, value_parser = clap::value_parser!(i8).range(-100..=100))]
    pub(crate) brightness: Option<i8>,
    /// Contrast (0-100).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub(crate) contrast: Option<u8>,
    /// Background brightness (-100 to 100).
    #[arg(long, value_parser = clap::value_parser!(i8).range(-100..=100))]
    pub(crate) bg_brightness: Option<i8>,
    /// Auto light: a low-light model that brightens dark frames. Needs an
    /// NPU; does nothing without one.
    #[arg(long)]
    pub(crate) auto_light: Option<bool>,
    /// How much of auto light's correction to apply (0-100).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub(crate) auto_strength: Option<u8>,
}

/// Shared --on / --off pair for effect subcommands.
#[derive(Debug, clap::Args)]
pub(crate) struct EnableFlags {
    /// Enable the effect.
    #[arg(long)]
    pub(crate) on: bool,
    /// Disable the effect.
    #[arg(long, conflicts_with = "on")]
    pub(crate) off: bool,
}

impl EnableFlags {
    pub(crate) fn resolve(&self) -> Option<bool> {
        if self.on {
            Some(true)
        } else if self.off {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum EffectArg {
    CenterStage,
    PortraitBlur,
    BgReplace,
    StudioLight,
    Reactions,
}

impl EffectArg {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::CenterStage => "center_stage",
            Self::PortraitBlur => "portrait_blur",
            Self::BgReplace => "bg_replace",
            Self::StudioLight => "studio_light",
            Self::Reactions => "reactions",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Zoom {
    Subtle,
    Normal,
    Tight,
}

impl Zoom {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Subtle => "subtle",
            Self::Normal => "normal",
            Self::Tight => "tight",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Mode {
    Single,
    Group,
}

impl Mode {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Group => "group",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Reaction {
    Heart,
    ThumbsUp,
    ThumbsDown,
    Tada,
    Clap,
    Wave,
    Joy,
    OpenMouth,
    Cry,
    Thinking,
}

impl Reaction {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Heart => "heart",
            Self::ThumbsUp => "thumbs_up",
            Self::ThumbsDown => "thumbs_down",
            Self::Tada => "tada",
            Self::Clap => "clap",
            Self::Wave => "wave",
            Self::Joy => "joy",
            Self::OpenMouth => "open_mouth",
            Self::Cry => "cry",
            Self::Thinking => "thinking",
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum CameraCommand {
    /// List available cameras (the active one is marked).
    List,
    /// Select a camera by id or /dev/videoN path.
    Select { id: String },
    /// Show the virtual-camera node info.
    Info,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AutostartCommand {
    /// Start the daemon automatically with the graphical session.
    Enable,
    /// Stop starting the daemon automatically.
    Disable,
    /// Show whether autostart is enabled and the daemon is running.
    Status,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use shared::dbus::{EFFECT_IDS, REACTION_IDS};

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("vinoopeneffects-ctl").chain(args.iter().copied()))
    }

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn effect_args_cover_every_daemon_effect() {
        let ids: Vec<_> = EffectArg::value_variants().iter().map(|e| e.id()).collect();
        assert_eq!(ids, EFFECT_IDS);
    }

    #[test]
    fn reaction_args_cover_every_daemon_reaction() {
        let ids: Vec<_> = Reaction::value_variants().iter().map(|r| r.id()).collect();
        assert_eq!(ids, REACTION_IDS);
    }

    #[test]
    fn zoom_and_mode_ids_match_config_values() {
        let zooms: Vec<_> = Zoom::value_variants().iter().map(|z| z.id()).collect();
        assert_eq!(zooms, ["subtle", "normal", "tight"]);
        let modes: Vec<_> = Mode::value_variants().iter().map(|m| m.id()).collect();
        assert_eq!(modes, ["single", "group"]);
    }

    #[test]
    fn enable_flags_resolve() {
        let flags = |on, off| EnableFlags { on, off }.resolve();
        assert_eq!(flags(true, false), Some(true));
        assert_eq!(flags(false, true), Some(false));
        assert_eq!(flags(false, false), None);
    }

    #[test]
    fn on_and_off_conflict() {
        assert!(parse(&["portrait-blur", "--on", "--off"]).is_err());
        assert!(parse(&["passthrough", "--on", "--off"]).is_err());
    }

    #[test]
    fn numeric_flags_are_range_checked() {
        assert!(parse(&["portrait-blur", "--strength", "100"]).is_ok());
        assert!(parse(&["portrait-blur", "--strength", "101"]).is_err());
        assert!(parse(&["center-stage", "--crop-top", "512"]).is_ok());
        assert!(parse(&["center-stage", "--crop-top", "513"]).is_err());
        assert!(parse(&["studio-light", "--brightness=-100"]).is_ok());
        assert!(parse(&["studio-light", "--brightness=-101"]).is_err());
        assert!(parse(&["studio-light", "--bg-brightness", "101"]).is_err());
    }

    #[test]
    fn kebab_case_values_parse_to_enums() {
        let cli = parse(&["enable", "bg-replace"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Enable {
                effect: EffectArg::BgReplace
            }
        ));

        let cli = parse(&["react", "thumbs-up"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::React {
                reaction: Reaction::ThumbsUp
            }
        ));

        assert!(parse(&["enable", "not-an-effect"]).is_err());
    }

    #[test]
    fn studio_light_auto_light_takes_an_explicit_bool() {
        let cli = parse(&["studio-light", "--auto-light", "false"]).unwrap();
        let Command::StudioLight(args) = cli.command else {
            panic!("expected studio-light");
        };
        assert_eq!(args.auto_light, Some(false));
    }
}
