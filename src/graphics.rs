//! The PC version's post-processing: every map has a "<Scene>_PC" prefab whose global
//! PostProcessVolume (Unity's Post Processing Stack v2) adds ambient occlusion, bloom,
//! chromatic aberration, colour grading, motion blur and lens distortion
//! ("DefaultGraphics_PC" for maps without their own, "Menu_Custom_PC" in the lobby).
//! export_postprocess.py -> maps/postprocess.json. Settings > Graphics switches them
//! (SettingsGraphics: Bloom, Motion Blur, Chromatic Aberration, Anti-Aliasing, V-Sync).
//!
//! The match has two cameras drawing into the same picture: the world, then the gun on top
//! (weapons.rs). Effects that read the scene's depth (ambient occlusion, motion blur, TAA)
//! go on the world camera; effects on the finished picture (bloom, aberration, colour,
//! lens, FXAA/SMAA) on the last camera, so they apply once, to the gun too.

use bevy::{
    anti_alias::{fxaa::Fxaa, smaa::Smaa, taa::TemporalAntiAliasing},
    camera::Hdr,
    pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel},
    post_process::{
        bloom::{Bloom, BloomCompositeMode, BloomPrefilter},
        effect_stack::{ChromaticAberration, LensDistortion},
        motion_blur::MotionBlur,
    },
    prelude::*,
    render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection, Msaa},
    window::{PresentMode, PrimaryWindow},
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::settings::{AntiAliasing, Settings};

/// One map's effects (only the values the profile overrides; the rest are PPv2 defaults).
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Profile {
    #[serde(rename = "AmbientOcclusion")]
    ao: Option<Effect>,
    #[serde(rename = "Bloom")]
    bloom: Option<Effect>,
    #[serde(rename = "ChromaticAberration")]
    aberration: Option<Effect>,
    #[serde(rename = "ColorGrading")]
    grading: Option<Effect>,
    #[serde(rename = "MotionBlur")]
    motion_blur: Option<Effect>,
    #[serde(rename = "LensDistortion")]
    lens: Option<Effect>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Effect {
    enabled: Option<bool>,
    active: Option<bool>,
    intensity: Option<f32>,
    threshold: Option<f32>,
    diffusion: Option<f32>,
    temperature: Option<f32>,
    #[serde(rename = "hueShift")]
    hue_shift: Option<f32>,
    saturation: Option<f32>,
    contrast: Option<f32>,
    brightness: Option<f32>,
    #[serde(rename = "shutterAngle")]
    shutter_angle: Option<f32>,
    #[serde(rename = "sampleCount")]
    sample_count: Option<u32>,
}

impl Effect {
    fn on(&self) -> bool {
        self.enabled.unwrap_or(true) && self.active.unwrap_or(true)
    }
}

fn effect(e: &Option<Effect>) -> Option<&Effect> {
    e.as_ref().filter(|e| e.on())
}

/// The profile of `map`: its own "<Scene>_PC", the lobby's Menu_Custom, else DefaultGraphics.
pub fn profile(data: &std::path::Path, map: &str) -> Profile {
    static ALL: std::sync::OnceLock<HashMap<String, Profile>> = std::sync::OnceLock::new();
    let all = ALL.get_or_init(|| {
        std::fs::read_to_string(data.join("maps/postprocess.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    });
    let key = if map.starts_with("lobby") { "Menu_Custom".to_string() } else { map.to_string() };
    all.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&key))
        .or_else(|| all.get_key_value("DefaultGraphics"))
        .map(|(_, p)| p.clone())
        .unwrap_or_default()
}

/// Bloom from PPv2's: intensity is how much of the bright part is added back, threshold is
/// in gamma space (the game renders in Unity's gamma colour space), diffusion how far it
/// spreads (1..10: 2-3 gives a tight glow around the bright pixels).
fn bloom(e: &Effect) -> Bloom {
    let intensity = e.intensity.unwrap_or(0.0);
    let threshold = e.threshold.unwrap_or(1.0).powf(2.2);
    let diffusion = e.diffusion.unwrap_or(7.0).clamp(1.0, 10.0);
    Bloom {
        // GUESS (by eye): PPv2's 1.5 on its tight, few-mip glow ~ 0.1 of Bevy's additive bloom
        intensity: (intensity * 0.12).min(0.5),
        low_frequency_boost: 0.0,
        high_pass_frequency: (diffusion / 10.0).clamp(0.15, 1.0),
        prefilter: BloomPrefilter { threshold, threshold_softness: 0.5 },
        composite_mode: BloomCompositeMode::Additive,
        ..Bloom::NATURAL
    }
}

/// PPv2 ColorGrading in LDR mode: saturation / contrast / brightness in -100..100,
/// temperature -100..100, hue shift in degrees.
fn grading(e: &Effect) -> ColorGrading {
    let pct = |v: Option<f32>| 1.0 + v.unwrap_or(0.0) / 100.0;
    let section = ColorGradingSection { saturation: pct(e.saturation), contrast: pct(e.contrast), ..default() };
    ColorGrading {
        global: ColorGradingGlobal {
            exposure: pct(e.brightness).max(0.01).log2(),
            temperature: e.temperature.unwrap_or(0.0) / 100.0,
            hue: e.hue_shift.unwrap_or(0.0).to_radians(),
            ..default()
        },
        shadows: section,
        midtones: section,
        highlights: section,
    }
}

#[allow(clippy::type_complexity)]
pub fn apply(
    mut commands: Commands,
    settings: Res<Settings>,
    data: Res<crate::DataDir>,
    map: Res<crate::map::CurrentMap>,
    cameras: Query<(Entity, &Camera), (With<Camera3d>, Without<crate::armory_preview::PreviewCamera>)>,
    added: Query<(), Added<Camera3d>>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
    mut last: Local<Option<(String, usize)>>,
) {
    let key = (map.0.clone(), cameras.iter().count());
    if !settings.is_changed() && added.is_empty() && last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    if let Ok(mut w) = window.single_mut() {
        let mode = if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
        if w.present_mode != mode {
            w.present_mode = mode;
        }
    }
    let p = profile(&data.0, &map.0);
    let first = cameras.iter().map(|(_, c)| c.order).min().unwrap_or(0);
    let top = cameras.iter().map(|(_, c)| c.order).max().unwrap_or(0);
    let aa = settings.anti_aliasing;
    for (e, cam) in &cameras {
        let mut ec = commands.entity(e);
        let world = cam.order == first;
        let finish = cam.order == top;
        ec.insert((Hdr, if aa == AntiAliasing::Msaa { Msaa::Sample4 } else { Msaa::Off }));
        // depth-based effects: on the world camera
        ec.remove::<(ScreenSpaceAmbientOcclusion, MotionBlur, TemporalAntiAliasing)>();
        if world {
            // (SSAO needs MSAA off: with MSAA anti-aliasing there is none)
            if settings.ambient_occlusion && aa != AntiAliasing::Msaa && effect(&p.ao).is_some() {
                ec.insert(ScreenSpaceAmbientOcclusion {
                    quality_level: ScreenSpaceAmbientOcclusionQualityLevel::Medium,
                    constant_object_thickness: 0.25,
                });
            }
            if let Some(m) = effect(&p.motion_blur).filter(|_| settings.motion_blur) {
                ec.insert(MotionBlur {
                    shutter_angle: m.shutter_angle.unwrap_or(270.0) / 360.0,
                    samples: m.sample_count.unwrap_or(10),
                    ..default()
                });
            }
            if aa == AntiAliasing::Taa {
                ec.insert(TemporalAntiAliasing::default());
            }
        }
        // effects on the finished picture: on the last camera
        ec.remove::<(Bloom, ChromaticAberration, LensDistortion, Fxaa, Smaa)>();
        ec.insert(ColorGrading::default());
        if finish {
            if let Some(b) = effect(&p.bloom).filter(|_| settings.bloom) {
                ec.insert(bloom(b));
            }
            if let Some(c) = effect(&p.aberration).filter(|_| settings.chromatic_aberration) {
                // PPv2: offset = intensity x 0.05 of the screen at the edges
                ec.insert(ChromaticAberration { intensity: c.intensity.unwrap_or(0.0) * 0.05, ..default() });
            }
            if let Some(g) = effect(&p.grading) {
                ec.insert(grading(g));
            }
            if let Some(l) = effect(&p.lens).filter(|_| settings.chromatic_aberration || settings.bloom) {
                // PPv2 -100..100 barrel/pincushion; GUESS scale for Bevy's
                ec.insert(LensDistortion { intensity: l.intensity.unwrap_or(0.0) / 100.0 * 0.5, ..default() });
            }
            match aa {
                AntiAliasing::Fxaa => {
                    ec.insert(Fxaa::default());
                }
                AntiAliasing::Smaa => {
                    ec.insert(Smaa::default());
                }
                _ => {}
            }
        }
    }
}
