//! Original IsViewTarget is core pawn state, retained independently of an animation instance.
use mh_character::{BoxWorld, CharacterSource, RecordsJson};
use mh_sim::{FighterDesc, Sim};
use std::{rc::Rc, sync::Arc};
const GS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";

#[test]
fn view_target_survives_animation_disabled_and_reenabled_for_both_perspectives() {
    let result = (|| -> Result<_, String> {
        let matrix=mh_spec::Spec::load(&mh_spec::Spec::default_dir(),false).map_err(|e|e.to_string())?;
        let vfs=Arc::new(mh_pak::Vfs::mount_default().map_err(|e|e.to_string())?);
        let loaded=mh_sim::load::load(&matrix,vfs.clone(),&[GS])?;
        let path=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json");
        let text=std::fs::read_to_string(path).map_err(|e|e.to_string())?;
        let records=RecordsJson(&text).load().map_err(|e|e.to_string())?;
        let mut sim=Sim::new(Rc::new(loaded.spec),Rc::new(loaded.geo),records,Box::new(BoxWorld::new()),1.0/60.0);
        let fi=sim.add_fighter(&FighterDesc{name:"local".into(),weapon:GS.into(),..Default::default()});
        sim.set_view_target(fi,true,true);
        assert!(sim.anim.is_none());
        assert!(sim.combat.fighters[fi].is_view_target);
        sim.enable_anim(mh_sim::animgraph::AnimAssets::new(vfs)?);
        assert!(sim.fanim[fi].is_view_target);
        for fp in [false,true] {
            sim.set_first_person(fi,fp);
            assert!(sim.combat.fighters[fi].is_view_target);
            assert!(sim.fanim[fi].is_view_target);
        }
        sim.respawn(fi,mordhau_core::ue::FVector::new(0.0,0.0,100.0),0.0);
        assert!(sim.combat.fighters[fi].is_view_target);
        assert!(sim.fanim[fi].is_view_target);
        assert!(sim.fanim[fi].view_target_debug_override);
        sim.anim=None;
        sim.set_view_target(fi,false,false);
        assert!(!sim.combat.fighters[fi].is_view_target);
        assert!(!sim.fanim[fi].is_view_target);
        sim.respawn(fi,mordhau_core::ue::FVector::new(0.0,0.0,100.0),0.0);
        assert!(!sim.combat.fighters[fi].is_view_target);
        Ok(())
    })();
    if let Err(e)=result {
        assert!(std::env::var("MORDHAU_GOLDEN_REQUIRED").as_deref()!=Ok("1"),"view-target fixture: {e}");
        eprintln!("SKIP view-target fixture: {e}");
    }
}
