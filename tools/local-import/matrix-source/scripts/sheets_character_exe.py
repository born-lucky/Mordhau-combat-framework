# sheets_character_exe.py - entity adapter (spec-kit speckit.adapters, kind "entity") for mh-character's exe-mode
# records the GDScript port has no reader for: horses, projectiles, the ladder mover (sheets r10).
#   input: godot/data_gen/spec_src/character_exe.json, written by `sh scripts/cargo.sh run -p mh-host --bin mh-spec-src`
#          (core/crates/mh-host/src/exe_records.rs dump(): mh-character's own loaders over extract/json -
#          HorseCfg::from_json + horse_character_records + new_horse (exe_horse.rs), ProjectileCfg::from_json_chain
#          (projectile.rs), the BP_LadderMover values new_ladder_mover sets (exe_ladder.rs))
#   output: ENT_HORSE_BP_Horse, ENT_PROJ_<class package>, ENT_LADDER_BP_LadderMover (+ the horse's FC_ curve entities)
# Field names are the Rust struct field names; the read-back (mh-host exe_records horse_cfg / projectile_cfg /
# ladder_mover) is proved equal to the loaders by core/crates/mh-host/tests/exe_records.rs.
# A missing dump skips the types (reported), like load_opt does for the Godot dumps.
VEC3 = {"soft_bubble_rel", "front_rel", "rear_rel", "attach_offset", "mesh_relative", "box_extent", "rotation_spin",
        "initial_velocity"}
VEC2 = {"min_z_distance_to_enter", "ranged_draw_turn_caps", "ranged_reload_turn_caps"}
INT_LIST = {"will_sticky_on", "will_pass_through_on"}
JSON = {"gears", "box_responses"}


def stype(k, v):
    if k in VEC3:
        return "vec3"
    if k in VEC2:
        return "vec2"
    if k in INT_LIST:
        return "int_list"
    if k in JSON:
        return "json"
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, int):
        return "int"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "string"
    if isinstance(v, list):
        return "float_list"
    raise SystemExit(f"character_exe.json: no spec type for {k}={v!r}")


def put(b, etype, eid, cfg):
    for k, v in cfg.items():
        t = stype(k, v)
        if t == "float_list":
            v = [float(x) for x in v]
        b.set(etype, eid, k, v, t)


def run(sp, b, ctx=None):
    d = sp.load_opt("character_exe")
    if not d:
        return
    reader = b.source("dump", "core/crates/mh-host/src/exe_records.rs", d["_reader"])
    # ---- horse -------------------------------------------------------------------------------------------------
    h = d["horse"]
    i = b.entity("horse", sp.ent_key(h["package"]), sp.ent_key(h["package"]),
                 b.source("pkg", h["package"], "extract/json/" + h["package"] + ".json"),
                 record="mh-character exe_horse.rs HorseCfg::from_json + horse_character_records + new_horse")
    put(b, "horse", i, h["cfg"])
    b.set("horse", i, "vehicle_package", h["vehicle_package"], "string")
    for field, path in sorted(h["curves"].items()):
        if not path:
            continue
        cid = sp.sid("ENT", b.codes["curve"], sp.ent_key(path))
        if cid not in b.entities:
            sp.put_curve(b, path)
        b.set("horse", i, field, cid, "ref", "curve")
    # ---- projectiles --------------------------------------------------------------------------------------------
    for p in d["projectiles"]:
        pkg = p["package"]
        i = b.entity("projectile", sp.ent_key(pkg), p["cfg"]["class"], b.source("pkg", pkg, "extract/json/" + pkg + ".json"),
                     record="mh-character projectile.rs ProjectileCfg::from_json_chain")
        b.set("projectile", i, "package", pkg, "string")
        b.set("projectile", i, "chain", list(p["chain"]), "string_list")
        put(b, "projectile", i, p["cfg"])
    # ---- equipment movement (every AMordhauEquipment Blueprint: melee, ranged, tools; rust-character r9 request) ----
    for p in d.get("equipment_movement", []):
        pkg = p["package"]
        i = b.entity("equipment_movement", sp.ent_key(pkg), p["cfg"]["class"],
                     b.source("pkg", pkg, "extract/json/" + pkg + ".json"),
                     record="mh-character equipment.rs EquipmentMovement::from_json_chain")
        b.set("equipment_movement", i, "package", pkg, "string")
        b.set("equipment_movement", i, "chain", list(p["chain"]), "string_list")
        b.set("equipment_movement", i, "native_root", p["native_root"], "string")
        put(b, "equipment_movement", i, p["cfg"])
    # ---- ladder mover -------------------------------------------------------------------------------------------
    l = d["ladder"]
    i = b.entity("ladder", sp.ent_key(l["package"]), sp.ent_key(l["package"]),
                 b.source("pkg", l["package"], "extract/json/" + l["package"] + ".json"),
                 record="mh-host exe_records.rs LadderMoverCfg (= exe_ladder.rs new_ladder_mover)")
    put(b, "ladder", i, l["cfg"])
    b.set("ladder", i, "vehicle_package", l["vehicle_package"], "string")
    return reader
