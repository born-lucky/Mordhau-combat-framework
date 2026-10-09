//! Original UBodySetup PhysXPC payload, located by serialization rather than magic scanning.
//! Proof: UBodySetup::Serialize 0x3306a20 and FPhysXCookingDataReader 0x332f640;
//! state/proofs/combat-contact-polish-20261008/WALL-COOKED-PROVIDER-SELECTED.md.
//! Only SHA1-verified, uncompressed installed pak entries enter the native loader.

use crate::{bulk, Bytes, Cursor, Reader};
use crate::asset::RF_CLASS_DEFAULT_OBJECT;
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct CookedBody {
    pub package: String,
    pub export: usize,
    pub properties: Map<String, Value>,
    pub payload: Bytes,
    pub sha1: [u8; 20],
    pub bulk: bulk::BulkHeader,
    pub tail_offset: i64,
    pub formats_end: i64,
}

fn verified(rd: &Reader, path: &str) -> Result<Bytes, String> {
    let (entry, _) = rd.vfs.entry(path).ok_or_else(|| format!("Missing cooked source {path}"))?;
    // Pak::read currently checks SHA1 only in its uncompressed branch. Do not widen that contract.
    if entry.method != 0 || entry.encrypted || entry.csize != entry.usize {
        return Err(format!("Cooked source requires an uncompressed, unencrypted pak entry: {path}"));
    }
    rd.vfs.try_read(path, true).map_err(|e| e.to_string())
}

fn bounded(r: &Cursor<'_>, end: i64) -> Result<(), String> {
    if r.bad || r.p < 0 || r.p > end { Err("Cooked BodySetup exceeds its export bounds".into()) } else { Ok(()) }
}
fn boolean(r: &mut Cursor<'_>, end: i64) -> Result<bool, String> {
    let value = r.s32();
    bounded(r, end)?;
    match value { 0 => Ok(false), 1 => Ok(true), _ => Err("Invalid cooked BodySetup archive boolean".into()) }
}

/// `mesh` is the exact StaticMesh object path (`package.export`), not a render LOD.
pub fn read(rd: &Reader, mesh: &str) -> Result<CookedBody, String> {
    let (package, mesh_export) = mesh.rsplit_once('.').ok_or("StaticMesh object export is required")?;
    let mesh_export: usize = mesh_export.parse().map_err(|_| "Invalid StaticMesh export")?;
    let pk = rd.try_open(package).map_err(|e| e.to_string())?;
    let head = verified(rd, &format!("{}{}", pk.name, pk.ext))?;
    if &*head != &*pk.uasset { return Err("Cached package differs from verified pak header".into()); }
    if !pk.uexp.is_empty() {
        let tail = verified(rd, &format!("{}.uexp", pk.name))?;
        if &*tail != &*pk.uexp { return Err("Cached package differs from verified pak exports".into()); }
    }
    let mesh_record = pk.exports.get(mesh_export).ok_or("StaticMesh export is out of range")?;
    let mesh_class = rd.node(&pk, mesh_record.cls).map(|n| rd.node_name(&n)).unwrap_or_default();
    if mesh_class != "StaticMesh" { return Err("Cooked collision owner is not a StaticMesh".into()); }
    let candidates: Vec<_> = pk.exports.iter().enumerate().filter(|(_, e)| {
        e.outer == mesh_export as i32 + 1 && rd.node(&pk, e.cls).is_some_and(|n| rd.node_name(&n) == "BodySetup")
    }).collect();
    if candidates.len() != 1 { return Err("StaticMesh must have one exact BodySetup child".into()); }
    let (export, e) = candidates[0];
    let end = e.off.checked_add(e.size).filter(|end| e.off >= 0 && e.size > 0 && *end <= pk.data_len() as i64)
        .ok_or("Invalid BodySetup export range")?;
    let mut r = pk.cursor(); r.p = e.off;
    let properties = rd.tagged(&pk, &mut r, end);
    bounded(&r, end)?;
    // `tagged` leaves the cursor immediately after the FName None terminator.
    if r.p < e.off + 8 || r.at(r.p - 8, 8).is_none() { return Err("Missing BodySetup tagged terminator".into()); }
    let mut terminator = pk.cursor(); terminator.p = r.p - 8;
    if pk.fname(&mut terminator) != "None" { return Err("Missing BodySetup tagged terminator".into()); }
    let tail_offset = r.p;
    if e.flags & RF_CLASS_DEFAULT_OBJECT == 0 && boolean(&mut r, end)? { r.skip(16); }
    r.skip(16); // BodySetupGuid, original Serialize before bCooked.
    bounded(&r, end)?;
    if !boolean(&mut r, end)? || !boolean(&mut r, end)? { return Err("BodySetup has no cooked physics data".into()); }
    let count = r.s32(); bounded(&r, end)?;
    if count <= 0 || count as i64 > (end - r.p) / 28 { return Err("Invalid BodySetup format count".into()); }
    let mut chosen = None;
    for _ in 0..count {
        let format = pk.fname(&mut r);
        let header = bulk::header(&pk, &mut r);
        bounded(&r, end)?;
        if format != "PhysXPC" { continue; }
        if chosen.is_some() { return Err("Duplicate PhysXPC cooked format".into()); }
        if header.count <= 0 || header.count != header.size || header.size > u32::MAX as i64 ||
            header.flags & (bulk::BULKDATA_UNUSED | bulk::BULKDATA_SERIALIZE_COMPRESSED_ZLIB) != 0 {
            return Err("Unsupported PhysXPC bulk descriptor".into());
        }
        // Verify the entire immutable backing entry before the existing ranged bulk reader uses it.
        if let Some((file, _)) = &header.file { verified(rd, file)?; }
        let payload = bulk::bytes(&rd.vfs, &pk, &header).ok_or("PhysXPC bulk payload is out of bounds")?;
        if payload.len() != header.size as usize { return Err("PhysXPC bulk length differs from its descriptor".into()); }
        let sha1 = sha1_smol::Sha1::from(&*payload).digest().bytes();
        chosen = Some((header, payload, sha1));
    }
    let (bulk, payload, sha1) = chosen.ok_or("BodySetup has no PhysXPC format")?;
    Ok(CookedBody { package: pk.name.clone(), export, properties, payload, sha1, bulk, tail_offset, formats_end: r.p })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_boolean_and_bounds_reject_invalid_input() {
        let mut r = Cursor::new(&[2, 0, 0, 0]);
        assert!(boolean(&mut r, 4).is_err());
        let mut r = Cursor::new(&[1, 0, 0]);
        assert!(boolean(&mut r, 3).is_err());
    }
}
