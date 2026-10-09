//! C ABI over mordhau-core (header: include/mordhau_core.h, hand-written; cbindgen is not installed).
//! The only `unsafe` in the workspace's sim crates lives here: raw pointers from the host.
//!
//! Lifecycle: mh_world_new(spec_json, dt) -> add fighters -> per frame: push inputs (applied on the next step, like
//! CombatState.input_*), mh_world_step, read state (mh_fighter_state or the JSON snapshot) -> mh_world_free.
//! Hosts: Godot GDExtension (r2), Bevy (mh-runtime links mordhau-core directly), Skyrim SKSE plugin (C++).
//! Not thread-safe: one world per thread.

use mordhau_core::combat::world::{Call, Input};
use mordhau_core::combat::World;
use mordhau_core::data::{RecordsJson, SpecSource};
use std::ffi::{c_char, CStr};
use std::rc::Rc;

pub struct MhWorld {
    w: World,
}

/// Plain per-fighter state for hosts (see mordhau_core.h)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MhFighterState {
    pub motion_kind: i32, // 0 Idle 1 Attack 2 Parry 3 Feinted 4 Blocked 5 Flinch 6 Stun 7 Disarmed
    pub stamina: i32,
    pub health: i32,
    pub dead: i32,
    pub start_time: f64,
    pub end_time: f64,
    pub attack_stage: i32,  // EAttackStage / EParryStage of the current attack / parry, -1 otherwise
    pub attack_move: i32,   // EAttackMove of the current attack, -1 otherwise
    pub attack_type: i32,   // EAttackType of the current attack, -1 otherwise
    pub movement_restriction: i32, // EMovementRestriction (GetMovementRestriction of the current motion)
    pub has_weapon: i32,
}

unsafe fn cstr<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    CStr::from_ptr(p).to_str().unwrap_or("")
}

unsafe fn world<'a>(w: *mut MhWorld) -> Option<&'a mut World> {
    w.as_mut().map(|x| &mut x.w)
}

fn name(w: &World, fi: i32) -> Option<String> {
    w.fighters.get(fi as usize).map(|f| f.name.clone())
}

/// Create a world from a spec JSON (the RecordsJson format, docs/RUST_CORE.md) and a fixed step in seconds.
/// Returns null when the spec does not parse.
///
/// # Safety
/// `spec_json` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn mh_world_new(spec_json: *const c_char, dt: f64) -> *mut MhWorld {
    let txt = cstr(spec_json);
    match RecordsJson(txt).load_spec() {
        Ok(spec) => Box::into_raw(Box::new(MhWorld { w: World::new(Rc::new(spec), dt) })),
        Err(_) => std::ptr::null_mut(),
    }
}

thread_local! {
    static LAST_ERROR: std::cell::RefCell<std::ffi::CString> = std::cell::RefCell::new(std::ffi::CString::default());
}

fn set_last_error(e: &str) {
    let c = std::ffi::CString::new(e.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|l| *l.borrow_mut() = c);
}

/// The last failure message of this thread ("" if none); valid until the next call on this thread.
#[no_mangle]
pub extern "C" fn mh_last_error() -> *const c_char {
    LAST_ERROR.with(|l| l.borrow().as_ptr())
}

/// Create a world from the user's install: the spec matrix (data_gen/spec or $MORDHAU_SPEC_DIR) + the paks, through
/// the shared host loader (mh_host::Data::load()?.combat_spec(weapons)); `weapons_json` = a JSON array of the weapon
/// Blueprint paths the fighters will hold (left-hand items included; the kick weapon is added). The golden
/// `mh_world_new(spec_json)` stays for the reference goldens. Returns null on failure (mh_last_error says why).
///
/// # Safety
/// `weapons_json` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn mh_world_new_from_install(weapons_json: *const c_char, dt: f64) -> *mut MhWorld {
    let built = (|| -> Result<_, String> {
        let ws: Vec<String> = serde_json::from_str(cstr(weapons_json)).map_err(|e| format!("weapons_json: {e}"))?;
        let refs: Vec<&str> = ws.iter().map(|s| s.as_str()).collect();
        mh_host::Data::load()?.combat_spec(&refs)
    })();
    match built {
        Ok(spec) => {
            set_last_error("");
            Box::into_raw(Box::new(MhWorld { w: World::new(Rc::new(spec), dt) }))
        }
        Err(e) => {
            set_last_error(&e);
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `w` must come from mh_world_new and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn mh_world_free(w: *mut MhWorld) {
    if !w.is_null() {
        drop(Box::from_raw(w));
    }
}

/// Add a fighter by weapon Blueprint path (left: "" or NULL = empty left hand). Returns its index, -1 on error.
///
/// # Safety
/// pointers must be valid NUL-terminated strings (left may be NULL).
#[no_mangle]
pub unsafe extern "C" fn mh_world_add_fighter(w: *mut MhWorld, name: *const c_char, weapon: *const c_char, left: *const c_char) -> i32 {
    let Some(w) = world(w) else { return -1 };
    let (n, wp, lp) = (cstr(name).to_string(), cstr(weapon).to_string(), cstr(left).to_string());
    if !w.spec.weapons.contains_key(&wp) || (!lp.is_empty() && !w.spec.weapons.contains_key(&lp)) {
        return -1;
    }
    w.add_fighter(&n, &wp, &lp) as i32
}

/// Input kinds of mh_world_push_input
pub const MH_INPUT_ATTACK: i32 = 0;
pub const MH_INPUT_FEINT: i32 = 1;
pub const MH_INPUT_PARRY: i32 = 2;
pub const MH_INPUT_RELEASE_BLOCK: i32 = 3;
pub const MH_INPUT_SWITCH_MODE: i32 = 4;
pub const MH_INPUT_TOGGLE_MODE: i32 = 5;

/// Queue an input for the next step. a = EAttackMove (attack) or EBlockType (parry); angle in degrees (attack).
/// Returns 0, or -1 on a bad fighter / kind.
///
/// # Safety
/// `w` must be a live world.
#[no_mangle]
pub unsafe extern "C" fn mh_world_push_input(w: *mut MhWorld, fighter: i32, kind: i32, a: i32, angle: f64) -> i32 {
    let Some(w) = world(w) else { return -1 };
    let Some(who) = name(w, fighter) else { return -1 };
    let ev = match kind {
        MH_INPUT_ATTACK => Input::Attack { who, mv: a as i64, angle },
        MH_INPUT_FEINT => Input::Feint { who },
        MH_INPUT_PARRY => Input::Parry { who, bt: a as i64 },
        MH_INPUT_RELEASE_BLOCK => Input::ReleaseBlock { who },
        MH_INPUT_SWITCH_MODE => Input::SwitchMode { who },
        MH_INPUT_TOGGLE_MODE => Input::ToggleMode { who },
        _ => return -1,
    };
    w.input(ev);
    0
}

/// Queue a contact (the attacker's weapon reached the defender's bone; host-side geometry) for the next step.
///
/// # Safety
/// `w` must be a live world; `bone` a NUL-terminated string (NULL = "Spine1").
#[no_mangle]
pub unsafe extern "C" fn mh_world_push_contact(w: *mut MhWorld, attacker: i32, defender: i32, bone: *const c_char) -> i32 {
    let Some(w) = world(w) else { return -1 };
    let (Some(a), Some(d)) = (name(w, attacker), name(w, defender)) else { return -1 };
    let b = if bone.is_null() { "Spine1".to_string() } else { cstr(bone).to_string() };
    w.input(Input::Contact { who: a, target: d, bone: b });
    0
}

/// Set a fighter's stamina before the next step (host hook; a test/scenario call)
///
/// # Safety
/// `w` must be a live world.
#[no_mangle]
pub unsafe extern "C" fn mh_world_set_stamina(w: *mut MhWorld, fighter: i32, v: i32) -> i32 {
    let Some(w) = world(w) else { return -1 };
    let Some(who) = name(w, fighter) else { return -1 };
    w.input(Input::Call(Call::SetStamina { who, v: v as i64 }));
    0
}

/// Advance one fixed step.
///
/// # Safety
/// `w` must be a live world.
#[no_mangle]
pub unsafe extern "C" fn mh_world_step(w: *mut MhWorld) {
    if let Some(w) = world(w) {
        w.step();
    }
}

/// World clock in seconds (tick_n x dt)
///
/// # Safety
/// `w` must be a live world.
#[no_mangle]
pub unsafe extern "C" fn mh_world_now(w: *mut MhWorld) -> f64 {
    world(w).map(|w| w.now).unwrap_or(0.0)
}

/// Fill `out` with a fighter's state. Returns 0, or -1 on a bad fighter.
///
/// # Safety
/// `w` must be a live world, `out` a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn mh_fighter_state(w: *mut MhWorld, fighter: i32, out: *mut MhFighterState) -> i32 {
    let Some(w) = world(w) else { return -1 };
    let Some(f) = w.fighters.get(fighter as usize) else { return -1 };
    if out.is_null() {
        return -1;
    }
    let fi = fighter as usize;
    let m = w.cur_m(fi).unwrap();
    let kinds = ["Idle", "Attack", "Parry", "Feinted", "Blocked", "Flinch", "Stun", "Disarmed"];
    let mut s = MhFighterState {
        motion_kind: kinds.iter().position(|k| *k == m.kind()).unwrap_or(0) as i32,
        stamina: f.stamina as i32,
        health: f.health as i32,
        dead: f.dead as i32,
        start_time: m.start_time,
        end_time: m.end_time,
        attack_stage: -1,
        attack_move: -1,
        attack_type: -1,
        movement_restriction: w.movement_restriction(fi) as i32,
        has_weapon: f.weapon.is_some() as i32,
    };
    if let Some(a) = m.attack() {
        s.attack_stage = a.stage as i32;
        s.attack_move = a.mv as i32;
        s.attack_type = a.ty as i32;
    } else if let Some(p) = m.parry() {
        s.attack_stage = p.stage as i32;
    }
    *out = s;
    0
}

unsafe fn write_buf(s: &str, buf: *mut c_char, cap: usize) -> usize {
    if !buf.is_null() && cap > s.len() {
        std::ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, s.len());
        *buf.add(s.len()) = 0;
    }
    s.len()
}

/// The full JSON snapshot of every fighter (mordhau_core::snapshot, the golden-trace format). Writes it when it fits
/// in `cap` bytes with its NUL and returns the length needed without the NUL.
///
/// # Safety
/// `w` must be a live world; `buf` valid for `cap` bytes (may be NULL when cap == 0).
#[no_mangle]
pub unsafe extern "C" fn mh_world_state_json(w: *mut MhWorld, buf: *mut c_char, cap: usize) -> usize {
    let Some(w) = world(w) else { return 0 };
    let s = mordhau_core::snapshot::row(w, w.hits.len(), w.events.len()).to_string();
    write_buf(&s, buf, cap)
}

/// Number of events emitted so far (hit / parry / chamber / motion / died / ...); mh_world_event_json reads one.
///
/// # Safety
/// `w` must be a live world.
#[no_mangle]
pub unsafe extern "C" fn mh_world_event_count(w: *mut MhWorld) -> usize {
    world(w).map(|w| w.hits.len()).unwrap_or(0)
}

/// Event `i` as JSON, same buffer contract as mh_world_state_json.
///
/// # Safety
/// `w` must be a live world; `buf` valid for `cap` bytes.
#[no_mangle]
pub unsafe extern "C" fn mh_world_event_json(w: *mut MhWorld, i: usize, buf: *mut c_char, cap: usize) -> usize {
    let Some(w) = world(w) else { return 0 };
    let Some(e) = w.hits.get(i) else { return 0 };
    let s = serde_json::Value::Object(e.clone()).to_string();
    write_buf(&s, buf, cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    /// Exercises every extern fn on the golden spec (skips without golden data): a Longsword strike lands on the
    /// defender in Release -> health drops, events come out as JSON.
    #[test]
    fn ffi_smoke() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/spec.json");
        let Ok(spec) = std::fs::read_to_string(&p) else {
            eprintln!("SKIP: no golden spec ({})", p.display());
            return;
        };
        let ls = CString::new("Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword").unwrap();
        unsafe {
            assert!(mh_world_new(CString::new("not json").unwrap().as_ptr(), 0.01).is_null());
            let w = mh_world_new(CString::new(spec).unwrap().as_ptr(), 0.002);
            assert!(!w.is_null());
            let a = mh_world_add_fighter(w, CString::new("A").unwrap().as_ptr(), ls.as_ptr(), std::ptr::null());
            let b = mh_world_add_fighter(w, CString::new("B").unwrap().as_ptr(), ls.as_ptr(), std::ptr::null());
            assert_eq!((a, b), (0, 1));
            assert_eq!(mh_world_add_fighter(w, CString::new("C").unwrap().as_ptr(), CString::new("nope").unwrap().as_ptr(), std::ptr::null()), -1);
            assert_eq!(mh_world_push_input(w, a, MH_INPUT_ATTACK, 0, 0.0), 0);
            assert_eq!(mh_world_push_input(w, 9, MH_INPUT_ATTACK, 0, 0.0), -1);
            let mut st = MhFighterState::default();
            let mut hit = false;
            for _ in 0..600 {
                mh_world_step(w);
                assert_eq!(mh_fighter_state(w, a, &mut st), 0);
                if st.motion_kind == 1 && st.attack_stage == 1 && !hit {
                    mh_world_push_contact(w, a, b, std::ptr::null());
                    hit = true;
                }
            }
            assert!(hit, "attack never reached Release");
            mh_fighter_state(w, b, &mut st);
            assert!(st.health < 100, "defender health {}", st.health);
            assert!(mh_world_now(w) > 1.0);
            let n = mh_world_state_json(w, std::ptr::null_mut(), 0);
            let mut buf = vec![0u8; n + 1];
            assert_eq!(mh_world_state_json(w, buf.as_mut_ptr() as *mut c_char, buf.len()), n);
            let v: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
            assert!(v["f"]["B"]["health"].as_i64().unwrap() < 100);
            let ne = mh_world_event_count(w);
            let kinds: Vec<String> = (0..ne)
                .map(|i| {
                    let l = mh_world_event_json(w, i, std::ptr::null_mut(), 0);
                    let mut b = vec![0u8; l + 1];
                    mh_world_event_json(w, i, b.as_mut_ptr() as *mut c_char, b.len());
                    serde_json::from_slice::<serde_json::Value>(&b[..l]).unwrap()["kind"].as_str().unwrap().to_string()
                })
                .collect();
            assert!(kinds.iter().any(|k| k == "hit"), "events {kinds:?}");
            assert_eq!(mh_world_set_stamina(w, a, 5), 0);
            mh_world_step(w);
            mh_fighter_state(w, a, &mut st);
            assert!(st.stamina >= 5);
            mh_world_free(w);
        }
    }

    /// mh_world_new_from_install: a world from the spec matrix + paks (skips without them); a strike lands as with
    /// the golden spec; a bad weapons_json gives null + a message in mh_last_error
    #[test]
    fn ffi_from_install() {
        unsafe {
            assert!(mh_world_new_from_install(CString::new("not json").unwrap().as_ptr(), 0.01).is_null());
            let msg = CStr::from_ptr(mh_last_error()).to_str().unwrap().to_string();
            assert!(msg.contains("weapons_json"), "last error: {msg}");
            let ls = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
            let w = mh_world_new_from_install(CString::new(format!("[\"{ls}\"]")).unwrap().as_ptr(), 0.002);
            if w.is_null() {
                eprintln!("SKIP: {}", CStr::from_ptr(mh_last_error()).to_str().unwrap());
                return;
            }
            let lsc = CString::new(ls).unwrap();
            assert_eq!(mh_world_add_fighter(w, CString::new("a").unwrap().as_ptr(), lsc.as_ptr(), std::ptr::null()), 0);
            assert_eq!(mh_world_add_fighter(w, CString::new("b").unwrap().as_ptr(), lsc.as_ptr(), std::ptr::null()), 1);
            for _ in 0..10 {
                mh_world_step(w);
            }
            assert!(mh_world_now(w) > 0.0);
            mh_world_free(w);
        }
    }
}
