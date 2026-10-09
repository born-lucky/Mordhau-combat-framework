//! Equivalence of the decoders against what CUE4Parse exported for the same packages: `mdx export` -> extract/gltf
//! (PNG, OGG, glb) and `mdx json` -> extract/json. Port of godot/tests/test_pak_assets.gd (same assets, same
//! tolerances). Needs the user's Mordhau install (mh-pak's default game folder or $MORDHAU_DIR) and extract/.

use mh_assets::pak_source::PakSource;
use mh_assets::texture::{self, Pixels, PixelFormat};
use mh_assets::{anim, coords, skeletal_mesh, sound, static_mesh};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const C: &str = "Mordhau/Content/";

fn src() -> &'static PakSource {
    static S: OnceLock<PakSource> = OnceLock::new();
    S.get_or_init(|| PakSource::mount_default().expect("mount the Mordhau paks"))
}

fn extract() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extract")
}

fn json_export(pkg: &str, typ: &str) -> Option<serde_json::Value> {
    let f = std::fs::read(extract().join("json").join(format!("{pkg}.json"))).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&f).ok()?;
    v.as_array()?.iter().find(|e| e["Type"] == typ).cloned()
}

fn files_under(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, ext, out);
        } else if p.to_string_lossy().ends_with(ext) {
            out.push(p);
        }
    }
}

/// "Mordhau/Content/.../X" of an extract/gltf file
fn pkg_of(root: &Path, f: &Path, ext: &str) -> String {
    let s = f.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
    s.trim_end_matches(ext).to_string()
}

// ---- textures ------------------------------------------------------------------------------------------------------

/// one or two per pixel format Mordhau's exported textures use (formats and sizes from their extract/json)
fn textures() -> Vec<String> {
    let castle = "Mordhau/Maps/AndrewG_Testing_Map/AG_Art/Assets/Castle/Castle2017/Castle_Materials/Textures/";
    vec![
        format!("{C}ForestCollection/Textures/TEX_ForestC_Temp_wsk"), // PF_B8G8R8A8 128
        format!("{C}Mordhau/UI/UIAssets/Textures/UI_VictoryRibbon-Secondary"), // PF_B8G8R8A8 171 (not a multiple of 4)
        format!("{C}ForestCollection/Textures/TEX_ForestC_Temp_col"), // PF_DXT5 128
        format!("{C}Mordhau/Assets/Weapons/Longbow/Arrow/LongbowArrow_RMA"), // PF_DXT5 128
        "Engine/Content/EngineResources/DefaultTexture".to_string(), // PF_DXT1 128
        format!("{C}{castle}romansarenafloor_RHAO"), // PF_DXT1 1024 (.ubulk)
        format!("{C}Mordhau/Assets/Environment/Wood/WoodRemake/Destructable/Cog/T_SailGradient"), // PF_G8 512
        format!("{C}ForestCollection/Textures/TEX_ForestC_Temp_nrm"), // PF_BC5 128
        format!("{C}{castle}overheadropeflags_basecolor_a"), // PF_BC7 1024x512
    ]
}

/// SizeX / SizeY / PixelFormat / PackedData (and the mip sizes) of the platform data equal the values CUE4Parse wrote,
/// for the test textures and every Texture2D package under Mordhau/UI (extract/manifest.tsv class column)
#[test]
fn texture_info_equals_json() {
    let mut pkgs = textures();
    let man = std::fs::read_to_string(extract().join("manifest.tsv")).unwrap();
    for l in man.lines() {
        let mut c = l.split('\t');
        let (p, _, cls) = (c.next().unwrap_or(""), c.next(), c.next().unwrap_or(""));
        if cls == "Texture2D" && p.starts_with("Mordhau/Content/Mordhau/UI/") && p.ends_with(".uasset") {
            pkgs.push(p.trim_end_matches(".uasset").to_string());
        }
    }
    let mut n = 0;
    for p in &pkgs {
        let Some(j) = json_export(p, "Texture2D") else { continue };
        let t = texture::info(src(), p, None).unwrap_or_else(|e| panic!("{p}: {e}"));
        assert_eq!(t.size_x as i64, j["SizeX"].as_i64().unwrap(), "{p} SizeX");
        assert_eq!(t.size_y as i64, j["SizeY"].as_i64().unwrap(), "{p} SizeY");
        assert_eq!(t.packed_data as i64, j["PackedData"].as_i64().unwrap(), "{p} PackedData");
        assert_eq!(t.pixel_format_name, j["PixelFormat"].as_str().unwrap(), "{p} PixelFormat");
        let jm = j["Mips"].as_array().unwrap();
        assert_eq!(t.mips.len(), jm.len(), "{p} mip count");
        for (m, w) in t.mips.iter().zip(jm) {
            assert_eq!((m.size_x as i64, m.size_y as i64), (w["SizeX"].as_i64().unwrap(), w["SizeY"].as_i64().unwrap()), "{p}");
            assert_eq!(m.bulk.size, w["BulkData"]["SizeOnDisk"].as_i64().unwrap(), "{p} mip size");
        }
        n += 1;
        if n % 200 == 0 {
            src().clear_cache();
        }
    }
    println!("  {n} textures");
    assert!(n > 100, "only {n} textures compared");
    src().clear_cache();
}

fn read_png(p: &Path) -> (usize, usize, Vec<u8>) {
    let mut d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).unwrap()));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&l| [l, l, l, 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        t => panic!("{}: png colour type {:?}", p.display(), t),
    };
    (w, h, rgba)
}

/// Mip 0 decoded from the paks equals the PNG CUE4Parse exported, within block-decoder rounding: per channel mean
/// |diff| <= 0.5 and max <= 8 (test_pak_assets.gd; Godot measured uncompressed, G8 and BC7 exact, DXT1/DXT5/BC5 max 1).
/// BC5 holds X/Y only: CUE4Parse writes a reconstructed Z into blue, so only R and G are compared.
#[test]
fn texture_pixels_equal_exported_png() {
    for p in textures() {
        let t = texture::info(src(), &p, None).unwrap();
        let fmt = t.format.unwrap();
        let data = texture::mip_data(src(), &t, 0).unwrap();
        let m = &t.mips[0];
        let Pixels::Rgba8(a) = texture::decode(fmt, m.size_x as usize, m.size_y as usize, &data).unwrap() else {
            panic!("{p}: HDR")
        };
        let (w, h, b) = read_png(&extract().join("gltf").join(format!("{p}.png")));
        assert_eq!((w, h), (m.size_x as usize, m.size_y as usize), "{p}: size");
        let chans = if fmt == PixelFormat::Bc5 { 2 } else { 4 };
        let (mut sum, mut worst) = (0u64, 0u8);
        for i in 0..a.len() {
            if i % 4 >= chans {
                continue;
            }
            let d = a[i].abs_diff(b[i]);
            sum += d as u64;
            worst = worst.max(d);
        }
        let mean = sum as f64 / (a.len() / 4 * chans) as f64;
        println!("  {} {:?} {}x{}: mean {:.3} max {}", p.rsplit('/').next().unwrap(), fmt, w, h, mean, worst);
        assert!(mean <= 0.5 && worst <= 8, "{p} ({fmt:?}): mean {mean:.3} max {worst}");
    }
    src().clear_cache();
}

// ---- sounds --------------------------------------------------------------------------------------------------------

/// Every exported sound: the OGG bytes in the SoundWave's format container are byte-identical to the .ogg CUE4Parse
/// wrote (it saves that bulk payload as is)
#[test]
fn sound_ogg_equals_exported() {
    let root = extract().join("gltf");
    let mut oggs = Vec::new();
    files_under(&root.join("Mordhau"), ".ogg", &mut oggs);
    let mut n = 0;
    for f in &oggs {
        let pkg = pkg_of(&root, f, ".ogg");
        let b = sound::ogg(src(), &pkg).unwrap_or_else(|e| panic!("{pkg}: {e}"));
        assert!(b == std::fs::read(f).unwrap(), "{pkg}: {} bytes from the paks differ from the exported .ogg", b.len());
        assert_eq!(&b[..4], b"OggS", "{pkg}");
        n += 1;
    }
    println!("  {n} sounds");
    assert!(n > 250, "only {n} sounds");
    src().clear_cache();
}

// ---- meshes --------------------------------------------------------------------------------------------------------

fn static_meshes() -> Vec<String> {
    [
        "Mordhau/Assets/Environment/Props/Buildings/Courtyard/SM_CY_RoofPlane",
        "Mordhau/Assets/Environment/Rocks/StaticMeshes/Scatter/SM_RockGraniteScatter_03",
        "Mordhau/Maps/Arena_Map/MapAssets/Meshes/arena_pillar_stand_01a",
        "Mordhau/Maps/Arena_Map/MapAssets/Meshes/arena_floor_bricks_01d",
        "Mordhau/Assets/Environment/Wood/WoodRemake/Destructable/CastleMetal_WoodDoor/destroy_doorcastle_01",
    ]
    .iter()
    .map(|s| format!("{C}{s}"))
    .collect()
}

/// the character body and the weapon meshes the port loads
fn skeletal_meshes() -> Vec<String> {
    vec![
        "Mordhau/Content/UMA/UMA/Master/UMA_Master".to_string(),
        format!("{C}Mordhau/Assets/Weapons/Longsword/Kickstarter_Longsword/SK_kslongsword_body"),
        format!("{C}Mordhau/Assets/Weapons/Longsword/Teutonic/SKeletal_Meshes/SK_TeutonicLongsword"),
        format!("{C}Mordhau/Assets/Weapons/Longsword/SkeletalMeshes/SK_Longsword_Blade_01"),
    ]
}

/// A triangle soup in Y-up metres: per surface positions, normals, indices, and (skinned) per-vertex
/// "bone:weight,..." influence keys
#[derive(Default)]
struct Surf {
    v: Vec<[f32; 3]>,
    n: Vec<[f32; 3]>,
    ix: Vec<u32>,
    inf: Vec<String>,
}

fn pkey(p: [f32; 3]) -> String {
    format!("{},{},{}", (p[0] * 10000.0).round() as i64, (p[1] * 10000.0).round() as i64, (p[2] * 10000.0).round() as i64)
}

/// Triangles as a multiset of position triples (corners to 0.1 mm, sorted), independent of vertex order and of how
/// the exporter split or merged vertices and primitives
fn tri_keys(s: &[Surf]) -> HashMap<String, i64> {
    let mut k = HashMap::new();
    for a in s {
        for t in a.ix.chunks_exact(3) {
            let mut c: Vec<String> = t.iter().map(|&i| pkey(a.v[i as usize])).collect();
            c.sort();
            *k.entry(c.join("|")).or_insert(0) += 1;
        }
    }
    k
}

fn tri_diff(a: &HashMap<String, i64>, b: &HashMap<String, i64>) -> i64 {
    let mut d: i64 = a.iter().map(|(k, n)| (n - b.get(k).copied().unwrap_or(0)).abs()).sum();
    d += b.iter().filter(|(k, _)| !a.contains_key(*k)).map(|(_, n)| n).sum::<i64>();
    d
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// sum over triangles of dot(cross(v1 - v0, v2 - v0), normal at v0): its sign says which way the corners wind
fn winding(s: &[Surf]) -> f64 {
    let mut w = 0.0;
    for a in s.iter().filter(|a| !a.n.is_empty()) {
        for t in a.ix.chunks_exact(3) {
            let (v0, v1, v2) = (a.v[t[0] as usize], a.v[t[1] as usize], a.v[t[2] as usize]);
            w += dot(cross(sub(v1, v0), sub(v2, v0)), a.n[t[0] as usize]) as f64;
        }
    }
    w
}

/// Our static mesh LOD 0 as surfaces (one per section, Y-up metres, UE corner order)
fn static_surfs(m: &static_mesh::StaticMesh) -> Vec<Surf> {
    let vx = &m.vertices;
    m.sections
        .iter()
        .map(|s| Surf {
            v: vx.positions.iter().map(|&p| coords::pos(p)).collect(),
            n: vx.normals.iter().map(|&n| coords::dir(n)).collect(),
            ix: m.indices[s.first_index as usize..(s.first_index + 3 * s.num_triangles) as usize].to_vec(),
            inf: Vec::new(),
        })
        .collect()
}

fn influence_key(bones: &[u16], w: &[f32], names: &[String]) -> String {
    let sum: f32 = w.iter().sum();
    let mut parts: BTreeMap<String, i64> = BTreeMap::new();
    for (b, x) in bones.iter().zip(w) {
        let q = (x / sum.max(1e-9) * 255.0).round() as i64;
        if q > 0 {
            *parts.entry(names[*b as usize].clone()).or_insert(0) += q;
        }
    }
    parts.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(",")
}

fn skel_surfs(m: &skeletal_mesh::SkeletalMesh) -> Vec<Surf> {
    let names: Vec<String> = m.ref_skeleton.bones.iter().map(|b| b.name.clone()).collect();
    let per = m.max_influences;
    let vx = &m.vertices;
    let inf: Vec<String> = (0..vx.positions.len())
        .map(|i| influence_key(&m.bones[i * per..(i + 1) * per], &m.weights[i * per..(i + 1) * per], &names))
        .collect();
    m.sections
        .iter()
        .map(|s| Surf {
            v: vx.positions.iter().map(|&p| coords::pos(p)).collect(),
            n: vx.normals.iter().map(|&n| coords::dir(n)).collect(),
            ix: m.indices[s.base_index as usize..(s.base_index + 3 * s.num_triangles) as usize].to_vec(),
            inf: inf.clone(),
        })
        .collect()
}

struct Glb {
    doc: gltf::Document,
    blob: Vec<u8>,
}

fn glb(pkg: &str) -> Glb {
    let p = extract().join("gltf").join(format!("{pkg}.glb"));
    let g = gltf::Gltf::from_slice(&std::fs::read(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    Glb { blob: g.blob.clone().expect("glb BIN chunk"), doc: g.document }
}

/// The exported glb's primitives as surfaces (positions/normals as stored: Y-up metres), influences named by the skin
fn glb_surfs(g: &Glb) -> Vec<Surf> {
    let joint_names: Vec<String> = g
        .doc
        .skins()
        .next()
        .map(|s| s.joints().map(|j| j.name().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for mesh in g.doc.meshes() {
        for prim in mesh.primitives() {
            let r = prim.reader(|_| Some(&g.blob[..]));
            let v: Vec<[f32; 3]> = r.read_positions().unwrap().collect();
            let n: Vec<[f32; 3]> = r.read_normals().map(|i| i.collect()).unwrap_or_default();
            let ix: Vec<u32> = r.read_indices().unwrap().into_u32().collect();
            let mut inf = Vec::new();
            if !joint_names.is_empty() {
                let mut bones: Vec<Vec<u16>> = vec![Vec::new(); v.len()];
                let mut wts: Vec<Vec<f32>> = vec![Vec::new(); v.len()];
                for set in 0..2 {
                    if let (Some(j), Some(w)) = (r.read_joints(set), r.read_weights(set)) {
                        for (i, (jj, ww)) in j.into_u16().zip(w.into_f32()).enumerate() {
                            bones[i].extend_from_slice(&jj);
                            wts[i].extend_from_slice(&ww);
                        }
                    }
                }
                inf = (0..v.len()).map(|i| influence_key(&bones[i], &wts[i], &joint_names)).collect();
            }
            out.push(Surf { v, n, ix, inf });
        }
    }
    out
}

/// LOD 0 decoded from the paks has the same triangles, in the same Y-up metres, as the .glb CUE4Parse exported, wound
/// the same way against the normals (UE corner order = glTF's counter-clockwise front)
#[test]
fn static_mesh_equals_exported_glb() {
    for p in static_meshes() {
        let m = static_mesh::lod0(src(), &p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let mine = static_surfs(&m);
        let theirs = glb_surfs(&glb(&p));
        let (a, b) = (tri_keys(&mine), tri_keys(&theirs));
        let (na, nb): (i64, i64) = (a.values().sum(), b.values().sum());
        let d = tri_diff(&a, &b);
        println!("  {}: {na} triangles from paks, {nb} in glb, {d} differ", p.rsplit('/').next().unwrap());
        assert!(na == nb && d == 0, "{p}: {na} triangles from paks, {nb} in the glb, {d} differ");
        let (wa, wb) = (winding(&mine), winding(&theirs));
        assert!(wa.signum() == wb.signum() && wa > 0.0, "{p}: winding {wa} vs glb {wb}");
    }
    src().clear_cache();
}

fn normals_by_pos(s: &[Surf]) -> HashMap<String, Vec<[f32; 3]>> {
    let mut out: HashMap<String, Vec<[f32; 3]>> = HashMap::new();
    for a in s.iter().filter(|a| !a.n.is_empty()) {
        // only vertices a triangle uses (a static section's surface lists the whole vertex buffer)
        let mut used = vec![false; a.v.len()];
        for &i in &a.ix {
            used[i as usize] = true;
        }
        for i in (0..a.v.len()).filter(|&i| used[i]) {
            out.entry(pkey(a.v[i])).or_default().push(a.n[i]);
        }
    }
    out
}

/// Vertex normals (TangentZ, 8-bit and high-precision 16-bit) equal the glb's: every vertex from the paks has a glb
/// vertex at the same position whose normal is within 1 degree
#[test]
fn mesh_normals_equal_exported_glb() {
    let cos1 = 1f32.to_radians().cos();
    let mut all = static_meshes();
    all.extend(skeletal_meshes());
    for p in all {
        let (mine, hp) = if p.contains("/UMA/") || p.contains("Weapons/Longsword") {
            let m = skeletal_mesh::lod0(src(), &p).unwrap();
            (skel_surfs(&m), m.vertices.high_precision_tangents)
        } else {
            let m = static_mesh::lod0(src(), &p).unwrap();
            (static_surfs(&m), m.vertices.high_precision_tangents)
        };
        let want = normals_by_pos(&glb_surfs(&glb(&p)));
        let have = normals_by_pos(&mine);
        assert!(!have.is_empty(), "{p}: no normals decoded");
        let mut bad = 0;
        for (k, ns) in &have {
            for n in ns {
                if !want.get(k).is_some_and(|ws| ws.iter().any(|w| dot(norm(*n), norm(*w)) > cos1)) {
                    bad += 1;
                }
            }
        }
        println!("  {}: high-precision {hp}, {} positions, {bad} normals off", p.rsplit('/').next().unwrap(), have.len());
        assert_eq!(bad, 0, "{p}: {bad} normals differ from the glb");
    }
    src().clear_cache();
}

/// High-precision tangents (FPackedRGBA16N): no exported mesh uses them (4 in the game do), so this one is checked
/// against its JSON (174 vertices, 2 UV sets) and its own geometry: unit normals, and >= 95% of the triangles wound
/// counter-clockwise about their corner normals (UE order after the Y/Z swap)
#[test]
fn high_precision_normals_consistent() {
    let p = format!("{C}Mordhau/Maps/Castello/Map_Assets/OptimizationMeshes/castle_wall_arch_pllr_02");
    let m = static_mesh::lod0(src(), &p).unwrap();
    let v = &m.vertices;
    assert!(v.high_precision_tangents, "{p}: not high precision");
    assert_eq!((v.positions.len(), v.normals.len(), v.uvs.len()), (174, 174, 2), "{p}");
    for n in &v.normals {
        assert!((dot(*n, *n).sqrt() - 1.0).abs() <= 2e-3, "{p}: normal {n:?} not unit length");
    }
    let (mut agree, mut total) = (0, 0);
    for s in static_surfs(&m) {
        for t in s.ix.chunks_exact(3) {
            let (v0, v1, v2) = (s.v[t[0] as usize], s.v[t[1] as usize], s.v[t[2] as usize]);
            let face = cross(sub(v1, v0), sub(v2, v0));
            if dot(face, face).sqrt() < 1e-9 {
                continue;
            }
            total += 1;
            let ns = [s.n[t[0] as usize], s.n[t[1] as usize], s.n[t[2] as usize]];
            if dot(face, [ns[0][0] + ns[1][0] + ns[2][0], ns[0][1] + ns[1][1] + ns[2][1], ns[0][2] + ns[1][2] + ns[2][2]]) > 0.0 {
                agree += 1;
            }
        }
    }
    println!("  {agree} / {total} triangles agree");
    assert!(total > 0 && agree as f64 >= 0.95 * total as f64, "{p}: only {agree} / {total}");
    src().clear_cache();
}

/// LOD 0 + reference skeleton from the paks against the glb: same triangles, same joint names and rest poses (1e-4 m,
/// |dot| >= 0.99999), and each vertex position carries the same bone influences (weights to 1/255)
#[test]
fn skeletal_mesh_equals_exported_glb() {
    for p in skeletal_meshes() {
        let m = skeletal_mesh::lod0(src(), &p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let mine = skel_surfs(&m);
        let g = glb(&p);
        let theirs = glb_surfs(&g);
        let (a, b) = (tri_keys(&mine), tri_keys(&theirs));
        assert_eq!(tri_diff(&a, &b), 0, "{p}: triangles differ from the glb");
        let skin = g.doc.skins().next().expect("skin");
        let mut bad_rest = 0;
        for j in skin.joints() {
            let name = j.name().unwrap_or("");
            let bi = m.ref_skeleton.find(name).unwrap_or_else(|| panic!("{p}: glb joint {name} not in the skeleton"));
            let (t, r, _) = j.transform().decomposed();
            let rest = m.ref_skeleton.pose[bi];
            let mt = coords::pos(rest.translation);
            let mq = coords::quat(rest.rotation);
            let d = sub(mt, t);
            let qd = (mq[0] * r[0] + mq[1] * r[1] + mq[2] * r[2] + mq[3] * r[3]).abs();
            if dot(d, d).sqrt() > 1e-4 || qd < 0.99999 {
                bad_rest += 1;
            }
        }
        assert_eq!(bad_rest, 0, "{p}: {bad_rest} joint rest poses differ");
        let by_pos = |s: &[Surf]| -> HashMap<String, String> {
            let mut o = HashMap::new();
            for a in s {
                for &i in &a.ix {
                    o.insert(pkey(a.v[i as usize]), a.inf[i as usize].clone());
                }
            }
            o
        };
        let (ia, ib) = (by_pos(&mine), by_pos(&theirs));
        let mut bad = 0;
        let mut first = String::new();
        for (k, w) in &ib {
            if ia.get(k) != Some(w) {
                bad += 1;
                if first.is_empty() {
                    first = format!("{k} pak {{{:?}}} glb {{{w}}}", ia.get(k));
                }
            }
        }
        println!("  {}: {} bones, {} triangles, {} vertex positions, {bad} influence sets differ",
            p.rsplit('/').next().unwrap(), m.ref_skeleton.bones.len(), a.len(), ib.len());
        assert_eq!(bad, 0, "{p}: first {first}");
    }
    src().clear_cache();
}

// ---- animations ----------------------------------------------------------------------------------------------------

const SKELETON: &str = "Mordhau/Content/UMA/UMA/Master/UMA_Master_Skeleton";

/// CUE4Parse's sampler (CAnimTrack.GetBoneTransform + GetKeyParams / FindTimeKey, CUE4Parse-Conversion/Writers/ActorX/
/// Structs/Animations/CAnimTrack.cs:31-160), which `mdx anim` used to bake the glb: key index and fraction at `frame`
fn cue_key(times: &[f32], n: usize, frame: f32, frames: i32) -> (usize, usize, f32) {
    if n <= 1 || frame == 0.0 {
        return (0, 0, 0.0);
    }
    if !times.is_empty() {
        let mut x = 0;
        for i in 0..n {
            if (frame - times[i]).abs() < 1e-8 {
                return (i, i, 0.0);
            }
            if frame < times[i] {
                x = i.saturating_sub(1);
                break;
            }
            x = i;
        }
        if x + 1 >= n {
            return (x, x, 0.0);
        }
        return (x, x + 1, (frame - times[x]) / (times[x + 1] - times[x]));
    }
    let pos = frame / frames as f32 * n as f32;
    let x = pos.floor() as usize;
    if x + 1 >= n {
        return (n - 1, n - 1, 0.0);
    }
    (x, x + 1, pos - x as f32)
}

/// Godot Quaternion.slerp (the GDScript test's interpolation between CUE4Parse keys)
fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let mut b = b;
    if d < 0.0 {
        d = -d;
        b = [-b[0], -b[1], -b[2], -b[3]];
    }
    let (s0, s1) = if 1.0 - d > 1e-6 {
        let om = d.acos();
        let so = om.sin();
        (((1.0 - t) * om).sin() / so, (t * om).sin() / so)
    } else {
        (1.0 - t, t)
    };
    [s0 * a[0] + s1 * b[0], s0 * a[1] + s1 * b[1], s0 * a[2] + s1 * b[2], s0 * a[3] + s1 * b[3]]
}

fn anim_glbs() -> (PathBuf, Vec<PathBuf>) {
    let root = extract().join("gltf");
    let mut g = Vec::new();
    files_under(&root.join(format!("{C}Mordhau/Animations")), ".glb", &mut g);
    g.sort();
    (root, g)
}

/// Every exported clip decodes (all AKF_PerTrackCompression, as extract/json says), and every 10th, sampled at each
/// glb key frame the way the exporter sampled (incl. its "Skeleton" translation retargeting onto UMA_Master), equals
/// the glb's rotations (|dot| >= 1 - 1e-5) and translations (1e-4 m = 0.1 mm)
#[test]
fn anim_sequence_equals_exported_glb() {
    let (root, glbs) = anim_glbs();
    let (src_skel, modes) = skeletal_mesh::skeleton(src(), SKELETON).unwrap();
    assert_eq!(modes.len(), src_skel.bones.len(), "BoneTree per bone");
    let mesh = skeletal_mesh::lod0(src(), "Mordhau/Content/UMA/UMA/Master/UMA_Master").unwrap();
    let tgt: HashMap<String, [f32; 3]> = mesh
        .ref_skeleton
        .bones
        .iter()
        .zip(&mesh.ref_skeleton.pose)
        .map(|(b, t)| (b.name.to_lowercase(), t.translation))
        .collect();
    let (mut clips, mut checked) = (0, 0);
    for (gi, gp) in glbs.iter().enumerate() {
        let pkg = pkg_of(&root, gp, ".glb");
        if let Some(j) = json_export(&pkg, "AnimSequence") {
            assert_eq!(j["CompressedDataStructure"]["KeyEncodingFormat"], "AKF_PerTrackCompression", "{pkg}");
        }
        let d = anim::decode(src(), &pkg).unwrap_or_else(|e| panic!("{pkg}: {e}"));
        if gi % 10 != 0 {
            continue;
        }
        let g = glb(&pkg);
        let fps = d.num_frames as f32 / d.sequence_length * d.rate_scale.max(1.0); // CAnimSequence.cs:42
        let mut by_bone: HashMap<String, &anim::Track> = HashMap::new();
        for t in &d.tracks {
            // beyond the reference skeleton: USkeleton VirtualBones, not in the mesh or the glb
            if let Some(b) = src_skel.bones.get(t.bone as usize) {
                by_bone.insert(b.name.to_lowercase(), t);
            }
        }
        let a = g.doc.animations().next().expect("animation");
        for ch in a.channels() {
            let bone = ch.target().node().name().unwrap_or("").to_lowercase();
            let t = *by_bone.get(&bone).unwrap_or_else(|| panic!("{pkg}: glb animates {bone}, no track in the paks"));
            let r = ch.reader(|_| Some(&g.blob[..]));
            let times: Vec<f32> = r.read_inputs().unwrap().collect();
            match r.read_outputs().unwrap() {
                gltf::animation::util::ReadOutputs::Rotations(rs) => {
                    for (tm, want) in times.iter().zip(rs.into_f32()) {
                        let fr = tm * fps;
                        if (fr - fr.round()).abs() > 1e-3 {
                            continue;
                        }
                        let frame = fr.round();
                        let (keys, frames) = match &t.rot {
                            Some(c) => (c.keys.clone(), c.frames.clone()),
                            None => (vec![[0.0, 0.0, 0.0, 1.0]], vec![]),
                        };
                        let (k0, k1, al) = cue_key(&frames, keys.len(), frame, d.num_frames);
                        let q = if al > 0.0 { slerp(keys[k0], keys[k1], al) } else { keys[k0] };
                        let got = coords::quat(q);
                        let dd = (got[0] * want[0] + got[1] * want[1] + got[2] * want[2] + got[3] * want[3]).abs();
                        assert!(dd >= 1.0 - 1e-5, "{pkg} {bone} rotation at frame {frame}: pak {got:?} glb {want:?}");
                        checked += 1;
                    }
                }
                gltf::animation::util::ReadOutputs::Translations(ts) => {
                    for (tm, want) in times.iter().zip(ts) {
                        let fr = tm * fps;
                        if (fr - fr.round()).abs() > 1e-3 {
                            continue;
                        }
                        let frame = fr.round();
                        let mode = modes.get(t.bone as usize).map(String::as_str).unwrap_or("");
                        let v = if mode.ends_with("::Skeleton") {
                            tgt[&bone]
                        } else if let Some(c) = &t.pos {
                            let (k0, k1, al) = cue_key(&c.frames, c.keys.len(), frame, d.num_frames);
                            let (p0, p1) = (c.keys[k0], c.keys[k1]);
                            if al > 0.0 { [p0[0] + (p1[0] - p0[0]) * al, p0[1] + (p1[1] - p0[1]) * al, p0[2] + (p1[2] - p0[2]) * al] } else { p0 }
                        } else {
                            [0.0; 3]
                        };
                        let got = coords::pos(v);
                        let dv = sub(got, want);
                        assert!(dot(dv, dv).sqrt() <= 1e-4, "{pkg} {bone} translation at frame {frame}: pak {got:?} glb {want:?}");
                        checked += 1;
                    }
                }
                _ => {}
            }
        }
        clips += 1;
    }
    println!("  {} clips decoded, {clips} compared, {checked} keys", glbs.len());
    assert!(glbs.len() > 100 && clips >= 10, "only {clips} clips");
    src().clear_cache();
}

/// The engine sampler (AEFPerTrackCompressionCodec::GetBoneAtomRotation VA 0x142e6b330 search + FastLerp) hits each
/// key exactly at its key time (k / (NumKeys - 1) * SequenceLength, or its frame over NumFrames - 1)
#[test]
fn anim_sampler_hits_keys_at_key_times() {
    let p = format!("{C}Mordhau/Animations/RawClips/1H/1H_RH_AltParry");
    let d = anim::decode(src(), &p).unwrap();
    let mut n = 0;
    for t in &d.tracks {
        let Some(c) = &t.rot else { continue };
        for k in 0..c.keys.len() {
            let tm = anim::key_time(&c.frames, c.keys.len(), k, d.num_frames, d.sequence_length);
            let (got, _, _) = d.sample(t, tm);
            let got = got.unwrap();
            let want = c.keys[k];
            let l = (want.iter().map(|x| x * x).sum::<f32>()).sqrt();
            let dd = (0..4).map(|i| got[i] * want[i] / l).sum::<f32>().abs();
            assert!(dd >= 1.0 - 1e-5, "{p} bone {} key {k} at {tm}: {got:?} vs {want:?}", t.bone);
            n += 1;
        }
    }
    println!("  {n} keys");
    assert!(n > 100);
}

/// Every LOD of static meshes (LODs > 0, mh-assets r4) against CUE4Parse's RenderData.LODs (extract/json): per LOD the
/// vertex count, UV channel count and sections, for the test meshes plus every StaticMesh the Arena map folder holds
#[test]
fn static_mesh_lods_equal_json() {
    let mut pkgs = static_meshes();
    let man = std::fs::read_to_string(extract().join("manifest.tsv")).unwrap();
    for l in man.lines() {
        let mut c = l.split('\t');
        let (p, _, cls) = (c.next().unwrap_or(""), c.next(), c.next().unwrap_or(""));
        // a StaticMesh package's first export is often its BodySetup (manifest class column)
        if (cls == "StaticMesh" || cls == "BodySetup") && p.starts_with("Mordhau/Content/Mordhau/Maps/Arena_Map/") && p.ends_with(".uasset") {
            pkgs.push(p.trim_end_matches(".uasset").to_string());
        }
    }
    let (mut meshes, mut n) = (0, 0);
    for p in &pkgs {
        let Some(j) = json_export(p, "StaticMesh") else { continue };
        let jl = j["RenderData"]["LODs"].as_array().unwrap();
        let ls = static_mesh::lods(src(), p).unwrap_or_else(|e| panic!("{p}: {e}"));
        assert_eq!(ls.len(), jl.len(), "{p}: LOD count");
        for (i, (m, w)) in ls.iter().zip(jl).enumerate() {
            let Some(m) = m else {
                assert!(w.get("VertexBuffer").is_none(), "{p} LOD {i}: cooked out here, not in json");
                continue;
            };
            assert_eq!(m.vertices.positions.len() as i64, w["VertexBuffer"]["NumVertices"].as_i64().unwrap(), "{p} LOD {i} vertices");
            assert_eq!(m.vertices.uvs.len() as i64, w["VertexBuffer"]["NumTexCoords"].as_i64().unwrap(), "{p} LOD {i} UVs");
            let ws = w["Sections"].as_array().unwrap();
            assert_eq!(m.sections.len(), ws.len(), "{p} LOD {i} sections");
            for (s, ws) in m.sections.iter().zip(ws) {
                assert_eq!(s.num_triangles as i64, ws["NumTriangles"].as_i64().unwrap(), "{p} LOD {i}");
                assert_eq!(s.first_index as i64, ws["FirstIndex"].as_i64().unwrap(), "{p} LOD {i}");
            }
            let tri: i64 = m.sections.iter().map(|s| s.num_triangles as i64).sum();
            assert!(m.indices.len() as i64 >= 3 * tri, "{p} LOD {i}: {} indices", m.indices.len());
            n += 1;
        }
        meshes += 1;
        src().clear_cache();
    }
    println!("  {meshes} meshes, {n} LODs = json");
    assert!(meshes > 20);
}

/// Skeletal LODs > 0 (mh-assets r5) against CUE4Parse's LODModels (extract/json): per LOD vertex count, UV channels,
/// sections (triangles, base index, vertices), for the test meshes plus every skeletal mesh of the longsword family
#[test]
fn skeletal_mesh_lods_equal_json() {
    let mut pkgs = skeletal_meshes();
    let man = std::fs::read_to_string(extract().join("manifest.tsv")).unwrap();
    for l in man.lines() {
        let mut c = l.split('\t');
        let (p, _, cls) = (c.next().unwrap_or(""), c.next(), c.next().unwrap_or(""));
        if cls == "SkeletalMesh" && p.starts_with("Mordhau/Content/Mordhau/Assets/Weapons/") && p.ends_with(".uasset") {
            pkgs.push(p.trim_end_matches(".uasset").to_string());
        }
    }
    let (mut meshes, mut n, mut cloth) = (0, 0, 0);
    for p in &pkgs {
        let Some(j) = json_export(p, "SkeletalMesh") else { continue };
        let jl = j["LODModels"].as_array().unwrap();
        let ls = match skeletal_mesh::lods(src(), p) {
            Ok(l) => l,
            Err(e) if e.0.contains("cloth") => {
                cloth += 1;
                continue;
            }
            Err(e) => panic!("{p}: {e}"),
        };
        assert_eq!(ls.len(), jl.len(), "{p}: LOD count");
        for (i, (m, w)) in ls.iter().zip(jl).enumerate() {
            let Some(m) = m else { continue };
            assert_eq!(m.vertices.positions.len() as i64, w["NumVertices"].as_i64().unwrap(), "{p} LOD {i} vertices");
            assert_eq!(m.vertices.uvs.len() as i64, w["NumTexCoords"].as_i64().unwrap(), "{p} LOD {i} UVs");
            let ws = w["Sections"].as_array().unwrap();
            assert_eq!(m.sections.len(), ws.len(), "{p} LOD {i} sections");
            for (s, ws) in m.sections.iter().zip(ws) {
                assert_eq!(s.num_triangles as i64, ws["NumTriangles"].as_i64().unwrap(), "{p} LOD {i}");
                assert_eq!(s.base_index as i64, ws["BaseIndex"].as_i64().unwrap(), "{p} LOD {i}");
            }
            n += 1;
        }
        meshes += 1;
        src().clear_cache();
    }
    println!("  {meshes} skeletal meshes, {n} LODs = json ({cloth} with cloth stopped)");
    assert!(meshes > 20);
}
