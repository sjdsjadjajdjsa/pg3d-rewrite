//! The original settings window (exported NGUI screen), shared by the lobby menu and the
//! in-match pause screen. We use its Multimedia tab: Music and Sound FX are the original
//! sliders, the brightness slider becomes mouse sensitivity. Changes apply live and are saved.

use bevy::prelude::*;

/// Marks background music players whose volume follows the Music slider.
#[derive(Component)]
pub struct Music;

/// Settings window: we use the Multimedia tab. Music and Sound FX are the original
/// sliders; the brightness slider becomes mouse sensitivity. Phone-only options are hidden.
pub fn setup_settings(s: &mut crate::ngui::NguiScreen, settings: &crate::settings::Settings) {
    let hide = [
        "3DTouchCont",
        "CheckBoxCont_4",
        "CheckBoxCont_5",
        "LobbyMusicToggleGroup",
        "FPSToggleCont",
        "ChatCont",
        "NotificaionsCont",
        "GS_StreamerCont",
        "CheckBox",
    ];
    for i in 0..s.nodes.len() {
        let name = s.nodes[i].name.clone();
        if hide.contains(&name.as_str()) {
            s.set_active(i, false);
        }
        // switched on by the game once the window has opened
        if name == "BtnBack" || (name == "Header_Label" && s.nodes[i].parent == Some(0)) {
            s.set_active(i, true);
        }
    }
    for (cont, value) in [("MusicCont", settings.music), ("SoundFXCont", settings.sfx), ("BrightnessCont", settings.sensitivity)] {
        let Some(c) = s.nodes.iter().position(|n| n.name == cont) else { continue };
        let prefix = s.path(c);
        let kids: Vec<usize> = (c..s.nodes.len()).filter(|&j| s.path(j).starts_with(&prefix)).collect();
        for j in kids {
            let n = s.nodes[j].name.clone();
            if s.nodes[j].slider.is_some() {
                s.set_slider(j, value);
            } else if n == "Amound_Label" {
                s.set_text(j, format!("{:.0}%", value * 100.0));
            } else if cont == "BrightnessCont" && (n == "NameSettings_Label" || n == "label_param") {
                s.set_text(j, if n == "label_param" { "Mouse" } else { "Mouse sensitivity" });
            }
        }
    }
}

/// Slider moved in the settings window: apply and save.
pub fn settings_sliders(
    mut moved: MessageReader<crate::ngui::NguiSlider>,
    mut screens: Query<&mut crate::ngui::NguiScreen>,
    mut settings: ResMut<crate::settings::Settings>,
    music: Query<&mut AudioSink, With<Music>>,
    mut cameras: Query<&mut crate::camera::PlayerCamera>,
) {
    let mut changed = false;
    let mut music = music;
    for m in moved.read() {
        let Ok(mut s) = screens.get_mut(m.screen) else { continue };
        let path = s.path(m.node);
        let (field, cont) = if path.contains("MusicCont") {
            (&mut settings.music, "MusicCont")
        } else if path.contains("SoundFXCont") {
            (&mut settings.sfx, "SoundFXCont")
        } else if path.contains("BrightnessCont") {
            (&mut settings.sensitivity, "BrightnessCont")
        } else {
            continue;
        };
        *field = m.value;
        changed = true;
        let label = (0..s.nodes.len()).find(|&j| s.nodes[j].name == "Amound_Label" && s.path(j).contains(cont));
        if let Some(j) = label {
            s.set_text(j, format!("{:.0}%", m.value * 100.0));
        }
    }
    if changed {
        crate::sound::set_sfx_volume(settings.sfx);
        for mut sink in &mut music {
            sink.set_volume(bevy::audio::Volume::Linear(settings.music));
        }
        for mut c in &mut cameras {
            c.sensitivity = settings.mouse_sensitivity();
        }
        settings.save();
    }
}

