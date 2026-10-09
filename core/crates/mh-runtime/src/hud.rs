//! hud.rs - HudPlugin. R1: a debug overlay (map, load progress, counts, sim tick). The UMG-laid-out HUD, kill feed,
//! scoreboard and BP_LocalPlay menu (godot/game/ui/**) are phase R5.

use crate::fighter::Fighter;
use crate::level::LevelState;
use crate::sim::Sim;
use bevy::prelude::*;

#[derive(Component)]
struct DebugText;

/// `show`: the debug overlay is on (`--debug-hud`); the game has no such overlay
pub struct HudPlugin {
    pub show: bool,
}

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        if self.show {
            app.add_systems(Startup, setup).add_systems(Update, update);
        }
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        DebugText,
        Text::new("mordhau (mh-runtime r1)"),
        TextFont { font_size: bevy::text::FontSize::Px(14.0), ..default() },
        Node { position_type: PositionType::Absolute, top: Val::Px(8.0), left: Val::Px(8.0), ..default() },
    ));
}

fn update(lvl: Res<LevelState>, sim: NonSend<Sim>, fighters: Query<(), With<Fighter>>, mut q: Query<&mut Text, With<DebugText>>) {
    let s = &lvl.stats;
    let status = if lvl.loading { "loading" } else if lvl.loaded { "loaded" } else { "idle" };
    let txt = format!(
        "mh-runtime r1 | {} [{}] {:.1}s\nmeshes {} + instances {} | primitives {} | materials {} | missing glb {}\nsim {} tick {} | fighters {}\nRMB look, WASD/QE move, Shift fast",
        lvl.map.rsplit('/').next().unwrap_or(""), status, lvl.load_secs,
        s.meshes_placed, s.ism_instances, s.primitives, s.materials, s.missing_glb,
        sim.0.name(), sim.0.ticks(), fighters.iter().count()
    );
    for mut t in q.iter_mut() {
        if t.0 != txt {
            t.0 = txt.clone();
        }
    }
}
