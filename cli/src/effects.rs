use futures_util::StreamExt;
use shared::dbus::{EFFECTS_INTERFACE, OBJECT_PATH, SERVICE_NAME, VariantMap, value_as_bool};
use zbus::{Connection, Proxy};
use zvariant::Value;

use crate::args::{CenterStageArgs, EffectArg, EnableFlags, Reaction, StudioLightArgs};
use crate::bus::effects_proxy;
use crate::values::{format_params, parse_value};

pub(crate) async fn list(conn: &Connection) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    let ids: Vec<String> = effects.call("ListEffects", &()).await?;
    for id in ids {
        let mark = if is_enabled(&effects, &id).await? {
            "on "
        } else {
            "off"
        };
        println!("[{mark}] {id}");
    }
    Ok(())
}

pub(crate) async fn enable(conn: &Connection, effect: EffectArg, on: bool) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    set_enabled(&effects, effect.id(), on).await?;
    show(&effects, effect.id()).await
}

pub(crate) async fn toggle(conn: &Connection, effect: EffectArg) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    let id = effect.id();
    let next = !is_enabled(&effects, id).await?;
    set_enabled(&effects, id, next).await?;
    println!("{id} {}", if next { "enabled" } else { "disabled" });
    Ok(())
}

pub(crate) async fn center_stage(conn: &Connection, args: CenterStageArgs) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    let id = EffectArg::CenterStage.id();
    apply_enabled(&effects, id, args.toggle.resolve()).await?;
    if let Some(zoom) = args.zoom {
        set_param(&effects, id, "zoom", zoom.id()).await?;
    }
    if let Some(mode) = args.mode {
        set_param(&effects, id, "mode", mode.id()).await?;
    }
    for (key, inset) in [
        ("crop_top", args.crop_top),
        ("crop_bottom", args.crop_bottom),
        ("crop_left", args.crop_left),
        ("crop_right", args.crop_right),
    ] {
        if let Some(inset) = inset {
            set_param(&effects, id, key, inset).await?;
        }
    }
    show(&effects, id).await
}

pub(crate) async fn portrait_blur(
    conn: &Connection,
    toggle: EnableFlags,
    strength: Option<u8>,
) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    let id = EffectArg::PortraitBlur.id();
    apply_enabled(&effects, id, toggle.resolve()).await?;
    if let Some(strength) = strength {
        set_param(&effects, id, "strength", u32::from(strength)).await?;
    }
    show(&effects, id).await
}

/// `none` clears the stored background and disables; any other value stores it and enables
/// unless `--off`.
pub(crate) async fn background(
    conn: &Connection,
    value: Option<String>,
    off: bool,
) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    let id = EffectArg::BgReplace.id();
    match value.as_deref() {
        Some("none") => {
            set_param(&effects, id, "background", "").await?;
            set_enabled(&effects, id, false).await?;
        }
        Some(background) => {
            set_param(&effects, id, "background", background).await?;
            set_enabled(&effects, id, !off).await?;
        }
        None if off => set_enabled(&effects, id, false).await?,
        None => {}
    }
    show(&effects, id).await
}

pub(crate) async fn studio_light(conn: &Connection, args: StudioLightArgs) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    let id = EffectArg::StudioLight.id();
    apply_enabled(&effects, id, args.toggle.resolve()).await?;
    if let Some(intensity) = args.intensity {
        set_param(&effects, id, "intensity", u32::from(intensity)).await?;
    }
    if let Some(brightness) = args.brightness {
        set_param(&effects, id, "brightness", i32::from(brightness)).await?;
    }
    if let Some(contrast) = args.contrast {
        set_param(&effects, id, "contrast", u32::from(contrast)).await?;
    }
    if let Some(bg_brightness) = args.bg_brightness {
        set_param(&effects, id, "bg_brightness", i32::from(bg_brightness)).await?;
    }
    if let Some(auto_light) = args.auto_light {
        set_param(&effects, id, "auto_light", auto_light).await?;
    }
    if let Some(auto_strength) = args.auto_strength {
        set_param(&effects, id, "auto_strength", u32::from(auto_strength)).await?;
    }
    show(&effects, id).await
}

pub(crate) async fn reactions(conn: &Connection, toggle: EnableFlags) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    let id = EffectArg::Reactions.id();
    apply_enabled(&effects, id, toggle.resolve()).await?;
    show(&effects, id).await
}

pub(crate) async fn react(conn: &Connection, reaction: Reaction) -> anyhow::Result<()> {
    effects_proxy(conn)
        .await?
        .call::<_, _, ()>("TriggerReaction", &(reaction.id()))
        .await?;
    println!("fired {}", reaction.id());
    Ok(())
}

/// Master passthrough: `Some(true)` bypasses all effects, `Some(false)` re-enables them, `None`
/// only reports the current state.
pub(crate) async fn passthrough(conn: &Connection, target: Option<bool>) -> anyhow::Result<()> {
    let effects = effects_proxy(conn).await?;
    if let Some(bypass) = target {
        effects.call::<_, _, ()>("SetBypass", &(bypass)).await?;
    }
    let bypassed: bool = effects.call("GetBypass", &()).await?;
    println!(
        "passthrough {}",
        if bypassed {
            "on (all effects bypassed)"
        } else {
            "off (effects active)"
        }
    );
    Ok(())
}

pub(crate) async fn set_raw_param(
    conn: &Connection,
    assignment: &str,
    raw_value: &str,
) -> anyhow::Result<()> {
    let (effect, key) = assignment
        .split_once('.')
        .ok_or_else(|| anyhow::anyhow!("expected EFFECT.KEY, e.g. studio_light.brightness"))?;
    let effects = effects_proxy(conn).await?;
    set_param(&effects, effect, key, Value::from(parse_value(raw_value))).await
}

pub(crate) async fn watch(conn: &Connection) -> anyhow::Result<()> {
    let proxy = Proxy::new(conn, SERVICE_NAME, OBJECT_PATH, EFFECTS_INTERFACE).await?;
    let effect_changed = proxy.receive_signal("EffectChanged").await?;
    let bypass_changed = proxy.receive_signal("BypassChanged").await?;
    let mut stream = futures_util::stream::select(effect_changed, bypass_changed);
    while let Some(message) = stream.next().await {
        let member = message
            .header()
            .member()
            .map(|m| m.to_string())
            .unwrap_or_default();
        let body = message.body();
        if member == "BypassChanged" {
            let on: bool = body.deserialize()?;
            println!("passthrough\t{}", if on { "on" } else { "off" });
        } else {
            let (id, params): (String, VariantMap) = body.deserialize()?;
            println!("{id}\t{}", format_params(&params));
        }
    }
    Ok(())
}

async fn show(effects: &Proxy<'_>, id: &str) -> anyhow::Result<()> {
    let params: VariantMap = effects.call("GetParams", &(id)).await?;
    println!("{id}: {}", format_params(&params));
    Ok(())
}

async fn is_enabled(effects: &Proxy<'_>, id: &str) -> anyhow::Result<bool> {
    let params: VariantMap = effects.call("GetParams", &(id)).await?;
    Ok(params
        .get("enabled")
        .and_then(value_as_bool)
        .unwrap_or(false))
}

async fn apply_enabled(effects: &Proxy<'_>, id: &str, on: Option<bool>) -> anyhow::Result<()> {
    match on {
        Some(on) => set_enabled(effects, id, on).await,
        None => Ok(()),
    }
}

async fn set_enabled(effects: &Proxy<'_>, id: &str, on: bool) -> anyhow::Result<()> {
    effects.call::<_, _, ()>("SetEnabled", &(id, on)).await?;
    Ok(())
}

async fn set_param<'v>(
    effects: &Proxy<'_>,
    id: &str,
    key: &str,
    value: impl Into<Value<'v>>,
) -> anyhow::Result<()> {
    let value: Value<'v> = value.into();
    effects
        .call::<_, _, ()>("SetParam", &(id, key, value))
        .await?;
    Ok(())
}
