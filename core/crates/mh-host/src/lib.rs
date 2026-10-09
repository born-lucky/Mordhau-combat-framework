//! mh-host: the shared host loader (sheets builder, r9). A host (the C ABI mordhau-core-ffi, mh-net-node, mh-runtime)
//! builds its world's data from the Spreadsheet Method spec matrix (data_gen/spec, mh-spec) + the user's own paks
//! (mh-pak), instead of the reference's golden record dump (core/tests/golden/spec.json), which stays for the
//! reference goldens only. One place, so every host loads the same data the same way (docs/SPEC_SHEETS.md "Consumers").
//!
//!   combat_spec(weapons)  mordhau-core's `Spec` (exe f32 records) through mh-sim's SpecBuilder: the matrix for every
//!                         record, the paks for curves / sockets / physics (proved equal to the golden dump by
//!                         mh-sim tests/sim.rs spec_matrix_equals_reference_records)
//!   mode_data(id)         mh-mode's `ModeData` (ModeData::from_spec, proved equal by mh-mode tests/spec_equiv.rs)
//!   kismet_json()         the Blueprint bytecode literals in the mode_kismet.json shape `{key: {value, src}}` that
//!                         `mh_mode::kismet::Kismet::from_json` reads, from the spec's ENT_CONST_KISMET_* rows
//!   character_records()   mh-character's CharacterRecords (SpecCharacter)
//!   horse_cfg() / projectile_cfg(class) / ladder_mover()  mh-character's horse / projectile / ladder-mover records
//!                         (exe_records.rs; == mh-character's loaders over extract/json: tests/exe_records.rs)
//!   Data::load()          the matrix + mounted paks, opened once and shared by the calls above
//! MORDHAU_SPEC_DIR / the mh-pak install lookup (Vfs::mount_default) choose the inputs.

pub mod exe_records;

use mordhau_core::data::Spec as CoreSpec;
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, String>;

/// The spec matrix + the mounted paks.
pub struct Data {
    pub matrix: mh_spec::Spec,
    pub vfs: Arc<mh_pak::Vfs>,
}

impl Data {
    /// data_gen/spec (or $MORDHAU_SPEC_DIR) and the user's install paks
    pub fn load() -> Result<Data> {
        let matrix = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).map_err(|e| format!("spec matrix: {e:?}"))?;
        let vfs = Arc::new(mh_pak::Vfs::mount_default().map_err(|e| format!("paks: {e:?}"))?);
        Ok(Data { matrix, vfs })
    }

    /// apply a mod layer (`{"ENT_*": {"FLD_*": value}}`, mh_spec::Spec::apply_overlay) before building
    pub fn with_overlay(mut self, layer: &Value) -> Result<Data> {
        self.matrix.apply_overlay(layer).map_err(|e| format!("overlay: {e:?}"))?;
        Ok(self)
    }

    /// the combat records for these weapon Blueprint paths (left-hand items included; the kick weapon is added by
    /// SpecBuilder)
    pub fn combat_spec(&self, weapons: &[&str]) -> Result<CoreSpec> {
        let rd = mh_pak::Reader::new(self.vfs.clone());
        mh_sim::spec::SpecBuilder::new(&self.matrix, &rd).build(weapons)
    }

    /// every weapon Blueprint path the matrix has (ENT_WPN_* class_path)
    pub fn weapon_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .matrix
            .entities_of("weapon")
            .filter_map(|id| self.matrix.str(id, "FLD_WPN_CLASS_PATH").ok().map(str::to_string))
            .collect();
        v.sort();
        v
    }

    /// mh-character's records (movement, move extra, character, physics, curves) from the matrix: SpecCharacter
    /// (mh-character src/spec_source.rs, == the record dump as f32: tests/spec_source.rs)
    pub fn character_records(&self) -> Result<mh_character::CharacterRecords> {
        use mh_character::CharacterSource;
        mh_character::spec_source::SpecCharacter(&self.matrix).load()
    }

    /// mh-mode's records of mode `id` (FFA / TDM / SKM / DU / TF / FL)
    pub fn mode_data(&self, id: &str) -> Result<mh_mode::data::ModeData> {
        mh_mode::data::ModeData::from_spec(&self.matrix, id)
    }

    /// the mode_kismet.json text (`{key: {value, src}}`) from the spec's KISMET constants
    pub fn kismet_json(&self) -> Result<String> {
        let mut out = Map::new();
        for id in self.matrix.entities_of("constant") {
            let Some(key) = id.strip_prefix("ENT_CONST_KISMET_") else { continue };
            let r = self.matrix.record(id).map_err(|e| format!("spec: {e:?}"))?;
            let value = r.json("literal").map_err(|e| format!("spec {id}: {e:?}"))?.clone();
            let src = r.str("bp_src").unwrap_or("");
            out.insert(key.to_string(), json!({"value": value, "src": src}));
        }
        Ok(Value::Object(out).to_string())
    }

    /// mh-character's HorseCfg (BP_Horse) from the matrix (exe_records.rs)
    pub fn horse_cfg(&self) -> Result<mh_character::exe_horse::HorseCfg> {
        exe_records::horse_cfg(&self.matrix)
    }

    /// a projectile class's ProjectileCfg (entity id or Blueprint package path) from the matrix
    pub fn projectile_cfg(&self, class: &str) -> Result<mh_character::projectile::ProjectileCfg> {
        exe_records::projectile_cfg(&self.matrix, class)
    }

    /// an equipment class's EquipmentMovement (entity id, package path or class stem such as "BP_Longbow")
    pub fn equipment_movement(&self, class: &str) -> Result<mh_character::equipment::EquipmentMovement> {
        exe_records::equipment_movement(&self.matrix, class)
    }

    /// the BP_LadderMover values new_ladder_mover sets, from the matrix
    pub fn ladder_mover(&self) -> Result<exe_records::LadderMoverCfg> {
        exe_records::ladder_mover(&self.matrix)
    }
}
