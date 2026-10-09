//! gameworld.rs - the map's gameplay actors through mh-world (owner: mh-world; host contract docs/RUST_RUNTIME.md
//! section 10): doors, destructibles, Frontline objectives (the wagon on its spline), ladders, spawners.
//!   - map load: `World::from_level` over the same LevelData mh-level reads for the map, `begin_play`
//!   - every frame: `tick(dt, queries, chars)` with the sim's fighters as CharViews, then the WorldEvents applied:
//!     component yaw / transform and actor transform -> the placed meshes' Transforms; Hidden / Destroyed -> hidden;
//!     MoveCharacter -> SimBackend::move_by (riders carried by the wagon); the rest counted (evidence)
//!   - the Use key (DefaultInput.ini "Use", E): UInteractionSystemComponent::GetInteractionTarget rva 0x14bd220 from
//!     the camera (mh_world::interaction::sweeps, sphere 30 on ECC channel 18 = GameTraceChannel5), then `interact`
//!   - Queries: a character overlaps an actor's component when its capsule (AMordhauCharacter: radius 50 = CMC ctor
//!     +0x45c, half height 96) overlaps that component's collision bodies (mh-level CollisionWorld) at the
//!     component's current transform: the capsule centre is carried into the component's load-time frame
//!     (bodies stay where the map placed them; rigid motion is undone on the query side).
//! Not done here (UNCONFIRMED / reported): the moving collision bodies for the sim's own movement (mh-level has no
//! body-transform API yet), Score / spawn / ladder-mount events, mh-mode's Frontline bridge.

use crate::level::{LevelState, UeMesh};
use bevy::prelude::*;
use mh_level::xf::Xf;
use mh_world::{ActorId, CharId, CharView, Queries, SpawnedStatus, WorldEvent};
use std::collections::{BTreeMap, HashMap};

/// AMordhauCharacter capsule (mh-character exe.rs: CapsuleRadius 50 +0x45c, CapsuleHalfHeight 96)
const CAPSULE_RADIUS: f64 = 50.0;
const CAPSULE_HALF_HEIGHT: f64 = 96.0;

pub struct GameWorld {
    pub map: String,
    pub w: mh_world::World,
    pub level: mh_level::LevelData,
    pub cw: mh_level::collision::CollisionWorld,
    /// actor -> its bodies (indices into cw.bodies)
    pub actor_bodies: HashMap<ActorId, Vec<u32>>,
    /// placement name -> (load-time world xf, current world xf)
    pub comp_xf: HashMap<String, (Xf, Xf)>,
    /// actor -> (load-time root xf, current root xf)
    pub actor_xf: HashMap<ActorId, (Xf, Xf)>,
    /// the interaction sweep channel's name (GameTraceChannel5)
    pub use_channel: String,
    pub events: BTreeMap<String, usize>,
    pub interactions: Vec<serde_json::Value>,
    pub last: Vec<String>,
    pub build_secs: f32,
}

/// the evidence of the world (dump_state "gameworld")
#[derive(Resource, Default, Clone, serde::Serialize)]
pub struct GameWorldStats {
    pub map: String,
    pub actors: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub events: BTreeMap<String, usize>,
    pub interactions: Vec<serde_json::Value>,
    pub last: Vec<String>,
    pub moved_components: usize,
    pub build_secs: f32,
    pub pushables: Vec<serde_json::Value>,
    pub doors: Vec<String>,
}

/// a pending interaction by name (script verb `world_use <actor substring>`), for offscreen evidence
#[derive(Resource, Default)]
pub struct UseRequest(pub Option<String>);

pub struct GameWorldPlugin;

impl Plugin for GameWorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameWorldStats>().init_resource::<UseRequest>().add_systems(Update, (build_world, tick_world).chain());
    }
}

fn v3(a: [f32; 3]) -> [f64; 3] {
    a.map(|x| x as f64)
}

fn forward_of(yaw_deg: f32) -> [f64; 3] {
    let y = (yaw_deg as f64).to_radians();
    [y.cos(), y.sin(), 0.0]
}

/// the yaw rotation as a UE quaternion (FRotator(0, yaw, 0).Quaternion())
fn yaw_q(yaw_deg: f64) -> [f64; 4] {
    let h = yaw_deg.to_radians() * 0.5;
    [0.0, 0.0, h.sin(), h.cos()]
}

/// translation / scale of an affine Xf (rotation dropped)
fn t_and_s(x: &Xf) -> ([f64; 3], [f64; 3]) {
    let m = &x.m;
    let col = |c: usize| (m[0][c] * m[0][c] + m[1][c] * m[1][c] + m[2][c] * m[2][c]).sqrt();
    ([m[0][3], m[1][3], m[2][3]], [col(0), col(1), col(2)])
}

impl GameWorld {
    fn char_views(sim: &crate::sim::Sim, me: u32) -> Vec<CharView> {
        sim.0
            .fighters()
            .iter()
            .map(|v| CharView {
                id: v.id as CharId,
                location: v3(v.loc),
                forward: forward_of(v.yaw),
                team: v.team.and_then(|t| u8::try_from(t).ok()).filter(|t| *t != 255),
                dead: v.health <= 0.0,
                allow_vehicles: true,
                has_player_controller: v.id == me,
                ..Default::default()
            })
            .collect()
    }
}

/// Queries over the current characters
struct Q<'a> {
    cw: &'a mh_level::collision::CollisionWorld,
    actor_bodies: &'a HashMap<ActorId, Vec<u32>>,
    comp_xf: &'a HashMap<String, (Xf, Xf)>,
    chars: &'a [CharView],
}

impl Q<'_> {
    /// does the character capsule overlap the actor's bodies (optionally only those of components whose name
    /// contains `part`), each at its component's current transform
    fn overlaps(&self, actor: ActorId, c: &CharView, part: Option<&str>) -> bool {
        let Some(bodies) = self.actor_bodies.get(&actor) else { return false };
        bodies.iter().any(|&b| {
            let body = &self.cw.bodies[b as usize];
            if part.is_some_and(|p| !body.name.contains(p)) {
                return false;
            }
            // carry the capsule centre into the body's load-time frame
            let p = match self.comp_xf.get(&body.name) {
                Some((init, cur)) => match cur.inverse() {
                    Some(inv) => (*init * inv).apply(c.location),
                    None => c.location,
                },
                None => c.location,
            };
            self.cw.overlap(p, CAPSULE_RADIUS, CAPSULE_HALF_HEIGHT, &|x| x == b)
        })
    }
}

impl Queries for Q<'_> {
    fn overlapping_chars(&self, actor: ActorId) -> Vec<CharId> {
        self.chars.iter().filter(|c| self.overlaps(actor, c, None)).map(|c| c.id).collect()
    }
    fn overlapping_actors(&self, _actor: ActorId) -> Vec<ActorId> {
        // UNCONFIRMED: actor-actor overlaps (destructible debris, pushable vs props) not tested yet
        Vec::new()
    }
    fn spawned_status(&self, _spawner: ActorId) -> SpawnedStatus {
        // the runtime spawns no equipment / vehicles yet
        SpawnedStatus::default()
    }
    /// APushableActor::PushArea: the BP's "Area" component (BP_SplinePushableActor Area_GEN_VARIABLE,
    /// CapZoneCylinderArea); falls back to the actor's bodies when the map carries no Area body
    fn area_chars(&self, actor: ActorId) -> Vec<CharId> {
        let has_area = self.actor_bodies.get(&actor).is_some_and(|b| b.iter().any(|&i| self.cw.bodies[i as usize].name.contains("Area")));
        let part = if has_area { Some("Area") } else { None };
        self.chars.iter().filter(|c| self.overlaps(actor, c, part)).map(|c| c.id).collect()
    }
    fn component_chars(&self, actor: ActorId, component: &str) -> Vec<CharId> {
        let has = self.actor_bodies.get(&actor).is_some_and(|b| b.iter().any(|&i| self.cw.bodies[i as usize].name.contains(component)));
        self.chars.iter().filter(|c| self.overlaps(actor, c, if has { Some(component) } else { None })).map(|c| c.id).collect()
    }
}

fn build_world(world: &mut World) {
    let (loaded, map) = {
        let l = world.resource::<LevelState>();
        (l.loaded, l.map.clone())
    };
    if !loaded || map.is_empty() {
        return;
    }
    if world.get_non_send_resource::<GameWorld>().is_some_and(|g| g.map == map) {
        return;
    }
    let Some(vfs) = world.resource::<crate::source::Source>().vfs.clone() else { return };
    let t0 = std::time::Instant::now();
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
    // the combat test level has no map package (level.rs TEST_LEVEL)
    let level = if map == crate::level::TEST_LEVEL { mh_level::LevelData::default() } else { mh_level::read(&pk, &map) };
    let mut w = mh_world::World::from_level(&pk, &level);
    // AMordhauGameMode ctor: HorseRespawnTime / BallistaRespawnTime / CatapultRespawnTime 30 (mh-world contract)
    w.game_mode_respawn = Some((30.0, 30.0, 30.0));
    // the session's rand() stream seed (sim.rs session_seed; mh-world and the bots draw from one engine stream, seeded
    // alike here: one shared CrtRand instance across the crates is UNCONFIRMED plumbing)
    if let Some(s) = world.get_resource::<crate::sim::SessionSeed>() {
        w.rng = mordhau_core::ue::CrtRand::new(s.0);
    }
    let cw = mh_level::collision::CollisionWorld::build(&pk, &level, None);
    let mut actor_bodies: HashMap<ActorId, Vec<u32>> = HashMap::new();
    for (b, a) in cw.body_actors(&level).into_iter().enumerate() {
        if let Some(a) = a {
            actor_bodies.entry(a).or_default().push(b as u32);
        }
    }
    let mut comp_xf = HashMap::new();
    for m in &level.meshes {
        comp_xf.insert(m.name.clone(), (m.xf, m.xf));
    }
    let mut actor_xf = HashMap::new();
    for (i, a) in level.gameplay.iter().enumerate() {
        if let Some(x) = a.xf {
            actor_xf.insert(i, (x, x));
        }
    }
    // ECC channel 18 (GetInteractionTarget's SweepMultiByChannel) = ECC_GameTraceChannel5 (ECollisionChannel order)
    let use_channel = mh_level::collision::Profiles::load(&vfs).channel_name("ECC_GameTraceChannel5");
    let mut gw = GameWorld {
        map: map.clone(),
        w,
        level,
        cw,
        actor_bodies,
        comp_xf,
        actor_xf,
        use_channel,
        events: BTreeMap::new(),
        interactions: Vec::new(),
        last: Vec::new(),
        build_secs: 0.0,
    };
    let chars = GameWorld::char_views(&world.non_send_resource::<crate::sim::Sim>(), 0);
    let ev = begin(&mut gw, &chars);
    gw.build_secs = t0.elapsed().as_secs_f32();
    world.insert_non_send_resource(gw);
    apply(world, ev);
    let mut st = world.resource_mut::<GameWorldStats>();
    st.map = map;
}

/// the Queries borrow the collision / transform fields, the World is borrowed mutably beside them
fn begin(gw: &mut GameWorld, chars: &[CharView]) -> Vec<WorldEvent> {
    let q = Q { cw: &gw.cw, actor_bodies: &gw.actor_bodies, comp_xf: &gw.comp_xf, chars };
    gw.w.begin_play(&q)
}

fn step(gw: &mut GameWorld, dt: f64, chars: &[CharView]) -> Vec<WorldEvent> {
    let q = Q { cw: &gw.cw, actor_bodies: &gw.actor_bodies, comp_xf: &gw.comp_xf, chars };
    gw.w.tick(dt, &q, chars)
}

fn tick_world(world: &mut World) {
    if world.get_non_send_resource::<GameWorld>().is_none() {
        return;
    }
    let dt = world.resource::<Time>().delta_secs() as f64;
    if dt <= 0.0 {
        return;
    }
    let me = world.get_resource::<crate::input::PlayerControl>().map(|p| p.id).unwrap_or(0);
    let chars = GameWorld::char_views(&world.non_send_resource::<crate::sim::Sim>(), me);
    // the Use key: DefaultInput.ini ActionMappings "Use" (E), or the script's world_use
    let mut target: Option<ActorId> = None;
    let pressed_use = {
        let keys = world.get_resource::<ButtonInput<KeyCode>>();
        let pc = world.get_resource::<crate::input::PlayerControl>();
        match (keys, pc) {
            (Some(k), Some(pc)) if pc.enabled => pc.actions.get("Use").is_some_and(|ks| ks.iter().any(|x| matches!(x, crate::input::Key::K(c) if k.just_pressed(*c)))),
            _ => false,
        }
    };
    let by_name = world.resource_mut::<UseRequest>().0.take();
    let me_view = chars.iter().find(|c| c.id == me).cloned();
    if let (Some(n), Some(_)) = (&by_name, &me_view) {
        let gw = world.non_send_resource::<GameWorld>();
        target = gw.w.actors.iter().find(|a| a.name.contains(n.as_str())).map(|a| a.id);
    }
    if pressed_use {
        if let Some(c) = &me_view {
            let cam = world.query_filtered::<&GlobalTransform, With<crate::camera::FlyCam>>().iter(world).next().copied();
            if let Some(g) = cam {
                let t = g.translation();
                let f = g.forward();
                let eye = [t.x as f64 * 100.0, t.z as f64 * 100.0, t.y as f64 * 100.0];
                let dir = [f.x as f64, f.z as f64, f.y as f64];
                let gw = world.non_send_resource::<GameWorld>();
                let body_actor = gw.cw.body_actors(&gw.level);
                let ch = gw.use_channel.clone();
                let mut hits = Vec::new();
                for (i, s) in mh_world::interaction::sweeps(eye, dir, [1.0; 3]).iter().enumerate() {
                    let filt = |b: u32| body_actor.get(b as usize).is_some_and(|a| a.is_some()) && gw.cw.bodies[b as usize].queries() && gw.cw.bodies[b as usize].response(&ch) != mh_level::collision::Resp::Ignore;
                    for h in gw.cw.sweep(s.start, s.end, mh_world::interaction::SWEEP_SPHERE_RADIUS, mh_world::interaction::SWEEP_SPHERE_RADIUS, &filt) {
                        if let Some(Some(a)) = body_actor.get(h.body as usize) {
                            hits.push(mh_world::interaction::SweepHit { sweep: i, actor: *a, point: h.impact_point });
                        }
                    }
                }
                target = gw.w.interaction_target(c, eye, dir, &hits);
            }
        }
    }
    let mut ev = Vec::new();
    if let (Some(a), Some(c)) = (target, &me_view) {
        let mut gw = world.non_send_resource_mut::<GameWorld>();
        let name = gw.w.actors[a].name.clone();
        let e = gw.w.interact(a, c);
        gw.interactions.push(serde_json::json!({"actor": name, "events": e.len(), "by": if by_name.is_some() { "script" } else { "Use key" }}));
        ev.extend(e);
    }
    {
        let mut gw = world.non_send_resource_mut::<GameWorld>();
        ev.extend(step(&mut gw, dt, &chars));
    }
    apply(world, ev);
}

/// the WorldEvents into the scene (and the sim for character moves)
fn apply(world: &mut World, ev: Vec<WorldEvent>) {
    if ev.is_empty() {
        sync_stats(world);
        return;
    }
    // which placements move: name -> new world xf
    let mut moved: HashMap<String, Xf> = HashMap::new();
    let mut hidden: Vec<String> = Vec::new();
    let mut carry: Vec<(u32, [f64; 3])> = Vec::new();
    {
        let mut gw = world.non_send_resource_mut::<GameWorld>();
        for e in &ev {
            let kind = format!("{e:?}").split([' ', '{', '(']).next().unwrap_or("").to_string();
            *gw.events.entry(kind.clone()).or_default() += 1;
            gw.last.push(format!("{e:?}").chars().take(160).collect());
            match e {
                WorldEvent::ComponentYaw { actor, component, yaw_deg } => {
                    // SetRelativeRotation(MakeRotator(0, 0, yaw)): the component's relative translation / scale kept
                    let root = gw.actor_xf.get(actor).map(|x| x.1).or(gw.level.gameplay[*actor].xf);
                    let comps = gw.level.gameplay[*actor].components.clone();
                    for c in comps.iter().filter(|c| c.rsplit('.').next() == Some(*component)) {
                        let (Some(root), Some((init, _))) = (root, gw.comp_xf.get(c).copied()) else { continue };
                        let init_root = gw.actor_xf.get(actor).map(|x| x.0).unwrap_or(root);
                        let Some(inv) = init_root.inverse() else { continue };
                        let (t, s) = t_and_s(&(inv * init));
                        let nw = root * Xf::trs(t, yaw_q(*yaw_deg), s);
                        gw.comp_xf.get_mut(c).unwrap().1 = nw;
                        moved.insert(c.clone(), nw);
                    }
                }
                WorldEvent::ComponentTransform { actor, component, rel } => {
                    let root = gw.actor_xf.get(actor).map(|x| x.1).or(gw.level.gameplay[*actor].xf);
                    let comps = gw.level.gameplay[*actor].components.clone();
                    for c in comps.iter().filter(|c| c.rsplit('.').next() == Some(*component)) {
                        let Some(root) = root else { continue };
                        let nw = root * *rel;
                        if let Some(x) = gw.comp_xf.get_mut(c) {
                            x.1 = nw;
                        }
                        moved.insert(c.clone(), nw);
                    }
                }
                WorldEvent::ActorTransform { actor, xf } => {
                    let Some((init_root, _)) = gw.actor_xf.get(actor).copied() else { continue };
                    gw.actor_xf.insert(*actor, (init_root, *xf));
                    let Some(inv) = init_root.inverse() else { continue };
                    let comps = gw.level.gameplay[*actor].components.clone();
                    for c in comps {
                        let Some((init, _)) = gw.comp_xf.get(&c).copied() else { continue };
                        let nw = *xf * (inv * init);
                        gw.comp_xf.get_mut(&c).unwrap().1 = nw;
                        moved.insert(c, nw);
                    }
                }
                WorldEvent::Hidden { actor } | WorldEvent::Destroyed { actor } => {
                    hidden.extend(gw.level.gameplay[*actor].components.iter().cloned());
                }
                WorldEvent::MoveCharacter { char, offset } => carry.push((*char, *offset)),
                _ => {}
            }
        }
        let n = gw.last.len();
        if n > 30 {
            gw.last.drain(..n - 30);
        }
    }
    if !moved.is_empty() || !hidden.is_empty() {
        let mut q = world.query::<(&UeMesh, &mut Transform, &mut Visibility)>();
        let mut n = 0;
        for (m, mut t, mut v) in q.iter_mut(world) {
            if let Some(x) = moved.get(&m.name) {
                *t = Transform::from_matrix(Mat4::from(crate::plan::xf_gltf(x)));
                n += 1;
            }
            if hidden.contains(&m.name) {
                *v = Visibility::Hidden;
            }
        }
        world.resource_mut::<GameWorldStats>().moved_components += n;
    }
    if !carry.is_empty() {
        let mut sim = world.non_send_resource_mut::<crate::sim::Sim>();
        for (c, off) in carry {
            sim.0.move_by(c, off.map(|x| x as f32));
        }
    }
    sync_stats(world);
}

fn sync_stats(world: &mut World) {
    let Some(gw) = world.get_non_send_resource::<GameWorld>() else { return };
    let mut by_kind = BTreeMap::new();
    for a in &gw.w.actors {
        let k = format!("{:?}", a.kind);
        *by_kind.entry(k.split([' ', '{', '(']).next().unwrap_or("").to_string()).or_insert(0usize) += 1;
    }
    let pushables: Vec<serde_json::Value> = gw
        .w
        .actors
        .iter()
        .filter(|a| gw.w.is_pushable_objective(a.id))
        .map(|a| {
            let cur = gw.actor_xf.get(&a.id).map(|x| (x.0.translation(), x.1.translation()));
            let bodies: Vec<&str> = gw.actor_bodies.get(&a.id).map(|b| b.iter().map(|&i| gw.cw.bodies[i as usize].name.as_str()).collect()).unwrap_or_default();
            serde_json::json!({"actor": a.name, "start_ue": cur.map(|c| c.0), "now_ue": cur.map(|c| c.1), "bodies": bodies})
        })
        .collect();
    let doors: Vec<String> = gw
        .w
        .actors
        .iter()
        .filter(|a| format!("{:?}", a.kind).starts_with("Door"))
        .map(|a| format!("{} @ {:?}", a.name, gw.actor_xf.get(&a.id).map(|x| x.1.translation().map(|v| v.round()))))
        .collect();
    let (events, interactions, last, actors, bs) = (gw.events.clone(), gw.interactions.clone(), gw.last.clone(), gw.w.actors.len(), gw.build_secs);
    let mut st = world.resource_mut::<GameWorldStats>();
    st.actors = actors;
    st.by_kind = by_kind;
    st.events = events;
    st.interactions = interactions;
    st.last = last;
    st.build_secs = bs;
    st.pushables = pushables;
    st.doors = doors;
}
