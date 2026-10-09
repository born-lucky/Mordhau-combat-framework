#![cfg(feature = "native-validation")]
//! Isolated guard fixture. Root dispatch and a reviewed guarded DLL are mandatory.
use mh_physics::{Scene, Transform};
use std::{path::PathBuf, sync::Arc};

const WALL: &str = "Mordhau/Content/Mordhau/Maps/Arena_Map/MapAssets/Meshes/Arena/arena_wall_01a.2";
const PAYLOAD_SHA1: [u8;20] = [0x64,0xd5,0x8d,0x13,0x50,0x39,0x0a,0x95,0xa9,0x69,0xe1,0x37,0xb5,0x92,0x56,0x00,0xb0,0x64,0xea,0x29];
const GEOMETRY_SHA1: [u8;20] = [0x5f,0xe3,0x53,0xaa,0x28,0x3d,0xf7,0x0f,0xd9,0x66,0xb6,0x41,0x35,0x4d,0x86,0xb1,0x46,0xa5,0xab,0x82];

#[test]
#[ignore = "Original cooked native entry: isolated reviewed guarded DLL and explicit root dispatch required"]
fn original_wall_full_arrays_rays_and_teardown() {
    let bridge = PathBuf::from(std::env::var_os("MH_PHYSX_VALIDATION_BRIDGE").expect("Reviewed guarded DLL path required"));
    assert!(bridge.is_absolute());
    let game = mh_pak::game_dir();
    let dlls = game.join("Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015");
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().unwrap()));
    let body = mh_pak::cooked_body::read(&rd, WALL).unwrap();
    assert_eq!(body.export,0); assert_eq!(body.tail_offset,13281);
    assert_eq!(body.payload.len(),395673); assert_eq!(body.sha1,PAYLOAD_SHA1);
    let placement = Transform { position: [-0.13617177,0.,0.], ..Default::default() };
    for repeat in 0..2 {
        let mut scene = Scene::open_for_validation(&bridge,&dlls,-980.,0.7,0.3,100.,1000.).unwrap();
        // Both rejects happen before the original deserializer is called.
        assert!(scene.load_cooked(&body.payload[..body.payload.len()-1],body.payload.len(),body.sha1).is_err());
        let mut changed = body.payload.to_vec(); changed[100]^=1;
        assert!(scene.load_cooked(&changed,body.payload.len(),body.sha1).is_err());
        let set = scene.load_cooked(&body.payload,body.payload.len(),body.sha1).unwrap();
        assert_eq!((set.first,set.normal,set.mirrored,set.triangles),(0,14,14,1));
        assert!(set.consumed > 13 && set.consumed as usize <= body.payload.len());
        let triangle = set.triangle_handles().next().unwrap();
        let info = scene.cooked_info(triangle).unwrap();
        assert_eq!((info.kind,info.vertices,info.triangles),(5,7564,14076));
        assert!(scene.cooked_readback_for_validation(triangle,7563,14076).is_err());
        let (vertices,indices,materials) = scene.cooked_readback_for_validation(triangle,7564,14076).unwrap();
        let mut hash = sha1_smol::Sha1::new();
        for v in &vertices { hash.update(&v.to_le_bytes()); }
        for i in &indices { hash.update(&i.to_le_bytes()); }
        for m in &materials { hash.update(&m.to_le_bytes()); }
        assert_eq!(hash.digest().bytes(),GEOMETRY_SHA1,"Full original native geometry differs from frozen parser");
        assert_eq!(materials.iter().filter(|&&m|m==0).count(),11456);
        assert_eq!(materials.iter().filter(|&&m|m==1).count(),2620);
        let hit = scene.ray_cooked(triangle,&placement,[1.;3],false,[-1365.,0.,175.],[0.,-1.,0.],8000.).unwrap().unwrap();
        assert_eq!((hit.face,hit.material),(8576,1));
        assert!((hit.position[1] - -1480.0319).abs()<0.001,"Original binary32 ray impact differs from static triangle anchor");
        // Original triangle centroid and geometric normal, frozen before native entry. The 10cm segment has one front and no back face.
        let normal = [-0.74474907_f32,0.6673446,0.];
        let front = [-1357.058_f32,-1463.6763,232.36365];
        let back = [-1349.6105_f32,-1470.3497,232.36365];
        let inward = [-normal[0],-normal[1],0.];
        assert_eq!(scene.ray_cooked(triangle,&placement,[1.;3],false,front,inward,10.).unwrap().unwrap().face,8576);
        assert!(scene.ray_cooked(triangle,&placement,[1.;3],false,back,normal,10.).unwrap().is_none());
        assert_eq!(scene.ray_cooked(triangle,&placement,[1.;3],true,back,normal,10.).unwrap().unwrap().face,8576);
        assert!(scene.ray_cooked(triangle,&placement,[1.;3],false,front,inward,1.).unwrap().is_none());
        // This ray meets the ORIGINAL convex hull but misses the ORIGINAL triangle mesh. No movement hull alias can pass.
        assert!(scene.ray_cooked(triangle,&placement,[1.;3],false,[-1900.,0.,50.],[0.,-1.,0.],2500.).unwrap().is_none());
        let simple_hit = (set.first..set.first+set.normal).any(|handle|
            scene.ray_cooked(handle,&placement,[1.;3],false,[-1900.,0.,50.],[0.,-1.,0.],2500.).unwrap().is_some());
        assert!(simple_hit,"Original convex-versus-triangle distinction prerequisite failed");
        assert!(scene.ray_cooked(triangle,&placement,[1.;3],false,[0.;3],[0.;3],10.).is_err());
        eprintln!("COOKED_GUARD repeat={repeat} counts14/14/1 consumed={} payload={} geometry={} face={} material={} bounds={:?}/{:?}",
                  set.consumed,body.payload.len(),hash.digest(),hit.face,hit.material,info.minimum,info.maximum);
        // Scene drop releases every native mesh, then original foundation. Guarded require_empty enforces live0.
        drop(scene);
        eprintln!("COOKED_GUARD repeat={repeat} teardown complete");
    }
}
