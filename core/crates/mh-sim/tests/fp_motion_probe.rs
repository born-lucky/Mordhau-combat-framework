//! fp-anim probe (run on demand: `cargo test -p mh-sim --test fp_motion_probe -- --ignored --nocapture`): the authored
//! motion of the 1P locomotion clips and the breathing additive, as RightHand relative to Spine1 in component space (the
//! 1P camera sits on Spine1 and turns with Position), to compare with the runtime's camera-frame probe
//! (scripts/runtime/fp_motion_probe.txt, dump_state rig.fp_probe).

use std::sync::Arc;

const GS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";

#[test]
#[ignore]
fn probe_1p_authored_hand_motion() {
    let Ok(m) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return };
    let vfs = Arc::new(mh_pak::Vfs::mount_default().unwrap());
    let ld = mh_sim::load::load(&m, vfs.clone(), &[GS]).unwrap();
    let sk = &ld.geo.skeleton;
    let a = mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap();
    let (rh, s1) = (sk.find("RightHand").unwrap(), sk.find("Spine1").unwrap());
    let base = "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/";
    for clip in ["2H_Sword_Idle_1P", "2H_Sword_Walk_1P", "2H_Sword_Run_1P"] {
        let path = format!("{base}{clip}");
        let Some(c) = a.clip(&path) else {
            let p2 = mh_sim::animgraph::AnimAssets::new(vfs.clone()).unwrap();
            let _ = p2;
            println!("{clip}: not found at {path}");
            continue;
        };
        let len = c.sequence_length as f64;
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        let n = 40;
        for k in 0..n {
            let t = len * k as f64 / n as f64;
            let cs = sk.to_component(&a.sample(sk, &path, t, true));
            let d = cs[rh].loc - cs[s1].loc;
            for (i, v) in [d.x, d.y, d.z].into_iter().enumerate() {
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v);
            }
        }
        println!("{clip}: length {len:.3} s, RateScale {}, RightHand - Spine1 range x {:.2} y {:.2} z {:.2} cm", c.rate_scale, hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
    }
    // the breathing additive over the idle clip's first frame, sampled over its length
    let add = "Mordhau/Content/Mordhau/Animations/RawClips/Misc/Atmospheric_Additive_1P";
    let idle = format!("{base}2H_Sword_Idle_1P");
    if let Some(c) = a.clip(add) {
        let len = c.sequence_length as f64;
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for k in 0..170 {
            let t = len * k as f64 / 170.0;
            let mut p = a.sample(sk, &idle, 0.0, true);
            let ad = mh_sim::additive::additive_sample(&a, sk, add, t);
            mh_sim::additive::apply_additive(&mut p, &ad, 1.0);
            let cs = sk.to_component(&p);
            let d = cs[rh].loc - cs[s1].loc;
            for (i, v) in [d.x, d.y, d.z].into_iter().enumerate() {
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v);
            }
        }
        println!("Atmospheric_Additive_1P over {len:.1} s: RightHand - Spine1 range x {:.2} y {:.2} z {:.2} cm", hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
    } else {
        println!("breathing additive not found at {add}");
    }
}
