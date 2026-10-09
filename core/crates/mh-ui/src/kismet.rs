//! Blueprint bytecode (Kismet) decoder for the widget / HUD Blueprints mh-ui runs, read straight from the paks.
//! Port of scripts/kismet/kis.py (itself CUE4Parse FKismetArchive.cs / KismetExpression.cs, UE 4.26 layout, with
//! rust-parity's EX_CallMulticastDelegate 0x63 fix) including its in-memory statement index (`at`: FName = 12 bytes in
//! memory, object pointer 8, FFieldPath 8) which EX_Jump / ubergraph entry points use - the same `@N` numbers as
//! state/ui_kismet/*.txt (scripts/ui_kismet.py), so a rule can be cited as "BP_MainMenu:Update@123".
//!
//! Besides the script, a UStruct export carries its ChildProperties (FField list: parameters, locals, members), parsed
//! here for parameter order / out flags and zero values (UE 4.26 FField::Serialize / FProperty::Serialize and the
//! per-type Serialize overrides, CUE4Parse FField.cs / FProperty.cs layout).

use mh_pak::asset::Package;
use mh_pak::Reader;
use std::collections::HashMap;
use std::rc::Rc;

/// An object a script refers to (FPackageIndex resolved): its package ("/Script/UMG", or a content path
/// "Mordhau/Content/..."), name, class name and outer name ("" when the outer is the package)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct ObjRef {
    pub package: String,
    pub name: String,
    pub class: String,
    pub outer: String,
}

/// EBlueprintTextLiteralType payloads
#[derive(Clone, Debug)]
pub enum TextLit {
    Empty,
    Localized(Box<Ex>, Box<Ex>, Box<Ex>),
    Invariant(Box<Ex>),
    Literal(Box<Ex>),
    StringTable(Box<Ex>, Box<Ex>),
}

/// EExprToken (UE 4.26 Script.h), one variant per token family
#[derive(Clone, Debug)]
pub enum Ex {
    /// EX_LocalVariable 0x00, EX_InstanceVariable 0x01, EX_DefaultVariable 0x02, EX_LocalOutVariable 0x48,
    /// EX_ClassSparseDataVariable 0x6C
    Var(u8, String),
    Return(Box<Ex>),
    Jump(u32),
    JumpIfNot(u32, Box<Ex>),
    Assert(Box<Ex>),
    /// tokens without payload (EX_Nothing, EX_EndOfScript, EX_Self, EX_True, ...), by token
    Op(u8),
    IntConst8(i64),
    /// EX_Let 0x0F (property, variable, value)
    Let(String, Box<Ex>, Box<Ex>),
    BitFieldConst(String, u8),
    /// EX_LetBool 0x14, EX_LetMulticastDelegate 0x43, EX_LetDelegate 0x44, EX_LetObj 0x5F, EX_LetWeakObjPtr 0x60
    LetKind(u8, Box<Ex>, Box<Ex>),
    /// EX_ClassContext 0x12, EX_Context 0x19, EX_Context_FailSilent 0x1A: object, skip, r-value property, expression
    Context(u8, Box<Ex>, u32, String, Box<Ex>),
    /// EX_MetaCast 0x13, EX_DynamicCast 0x2E, EX_ObjToInterfaceCast 0x52, EX_CrossInterfaceCast 0x54,
    /// EX_InterfaceToObjCast 0x55
    Cast(u8, Option<ObjRef>, Box<Ex>),
    Skip(u32, Box<Ex>),
    /// EX_VirtualFunction 0x1B, EX_LocalVirtualFunction 0x45 (by name)
    Virtual(u8, String, Vec<Ex>),
    /// EX_FinalFunction 0x1C, EX_LocalFinalFunction 0x46, EX_CallMath 0x68 (by function object)
    Final(u8, Option<ObjRef>, Vec<Ex>),
    Int(i64),
    Float(f64),
    Str(String),
    Obj(Option<ObjRef>),
    Name(String),
    /// EX_RotationConst 0x22 (pitch, yaw, roll), EX_VectorConst 0x23, 0x41
    Vec3(u8, [f64; 3]),
    Byte(u8),
    Text(TextLit),
    Transform([f64; 10]),
    StructConst(Option<ObjRef>, Vec<Ex>),
    SetArray(Box<Ex>, Vec<Ex>),
    PropertyConst(String),
    Int64(i64),
    UInt64(u64),
    Double(f64),
    PrimitiveCast(u8, Box<Ex>),
    /// EX_SetSet 0x39 / EX_SetMap 0x3B (target, items)
    SetColl(u8, Box<Ex>, Vec<Ex>),
    /// EX_SetConst 0x3D / EX_MapConst 0x3F / EX_ArrayConst 0x65 (inner property, items)
    ConstColl(u8, String, Vec<Ex>),
    StructMember(String, Box<Ex>),
    InstanceDelegate(String),
    PushFlow(u32),
    ComputedJump(Box<Ex>),
    PopFlowIfNot(Box<Ex>),
    InterfaceContext(Box<Ex>),
    SkipOffsetConst(u32),
    /// EX_AddMulticastDelegate 0x5C, EX_RemoveMulticastDelegate 0x62 (delegate, value)
    MultiOp(u8, Box<Ex>, Box<Ex>),
    ClearMulticast(Box<Ex>),
    BindDelegate(String, Box<Ex>, Box<Ex>),
    CallMulticast(Option<ObjRef>, Box<Ex>, Vec<Ex>),
    LetPersistent(String, Box<Ex>),
    SoftObjectConst(Box<Ex>),
    Switch(u32, Box<Ex>, Vec<(Ex, Ex)>, Box<Ex>),
    Instrumentation,
    ArrayGetByRef(Box<Ex>, Box<Ex>),
    FieldPathConst(Box<Ex>),
}

/// A statement: in-memory index + expression
pub type Stmt = (u32, Ex);

/// One FProperty of a struct / function / class (ChildProperties)
#[derive(Clone, Debug)]
pub struct Prop {
    pub name: String,
    /// "IntProperty", "StructProperty", ...
    pub ty: String,
    pub flags: u64,
    /// StructProperty struct / Object/Class property class / Byte/Enum enum (resolved)
    pub sub: Option<ObjRef>,
    /// ArrayProperty inner / SetProperty element / MapProperty key, value
    pub inner: Vec<Prop>,
}

/// EPropertyFlags (UE 4.26 ObjectMacros.h)
pub const CPF_PARM: u64 = 0x80;
pub const CPF_OUT_PARM: u64 = 0x100;
pub const CPF_RETURN_PARM: u64 = 0x400;
pub const CPF_REFERENCE_PARM: u64 = 0x8000000;

#[derive(Debug, Default)]
pub struct Func {
    pub name: String,
    pub flags: u32,
    pub props: Vec<Prop>,
    pub code: Vec<Stmt>,
    /// in-memory statement index -> position in `code`
    pub index: HashMap<u32, usize>,
    /// the export's SuperStruct (a parent class's function this one overrides)
    pub sup: Option<ObjRef>,
}

impl Func {
    pub fn params(&self) -> impl Iterator<Item = &Prop> {
        self.props.iter().filter(|p| p.flags & CPF_PARM != 0)
    }
}

/// "/Game/X" -> "Mordhau/Content/X" (the pak spelling); "/Script/.." kept
pub fn content_path(p: &str) -> String {
    if let Some(r) = p.strip_prefix("/Game/") {
        format!("Mordhau/Content/{r}")
    } else if let Some(r) = p.strip_prefix("/Engine/") {
        format!("Engine/Content/{r}")
    } else {
        p.to_string()
    }
}

/// Resolve FPackageIndex `i` of `pk`
pub fn resolve(pk: &Package, i: i32) -> Option<ObjRef> {
    if i < 0 {
        let im = pk.imports.get((-i - 1) as usize)?;
        let mut outer = String::new();
        let mut o = im.outer;
        let mut package = String::new();
        let mut first = true;
        let mut guard = 0;
        while o < 0 && guard < 16 {
            let x = pk.imports.get((-o - 1) as usize)?;
            if x.class_name == "Package" {
                package = content_path(&x.name);
                break;
            }
            if first {
                outer = x.name.clone();
                first = false;
            }
            o = x.outer;
            guard += 1;
        }
        if im.class_name == "Package" {
            package = content_path(&im.name);
        }
        Some(ObjRef { package, name: im.name.clone(), class: im.class_name.clone(), outer })
    } else if i > 0 {
        let ex = pk.exports.get((i - 1) as usize)?;
        let class = resolve(pk, ex.cls).map(|c| c.name).unwrap_or_default();
        let outer = if ex.outer > 0 { pk.exports.get((ex.outer - 1) as usize).map(|o| o.name.clone()).unwrap_or_default() } else { String::new() };
        Some(ObjRef { package: pk.name.clone(), name: ex.name.clone(), class, outer })
    } else {
        None
    }
}

struct K<'a> {
    pk: &'a Package,
    b: &'a [u8],
    o: usize,
    mi: u32,
}

#[derive(Debug)]
struct Bad;

impl<'a> K<'a> {
    fn take(&mut self, n: usize, mem: Option<u32>) -> Result<&'a [u8], Bad> {
        if self.o + n > self.b.len() {
            return Err(Bad);
        }
        let s = &self.b[self.o..self.o + n];
        self.o += n;
        self.mi += mem.unwrap_or(n as u32);
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Bad> {
        Ok(self.take(1, None)?[0])
    }
    fn u16(&mut self) -> Result<u16, Bad> {
        let s = self.take(2, None)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn i32(&mut self) -> Result<i32, Bad> {
        let s = self.take(4, None)?;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u32(&mut self) -> Result<u32, Bad> {
        Ok(self.i32()? as u32)
    }
    fn f32(&mut self) -> Result<f64, Bad> {
        let s = self.take(4, None)?;
        Ok(mh_pak::buf::short32(f32::from_le_bytes([s[0], s[1], s[2], s[3]])))
    }
    fn i64(&mut self) -> Result<i64, Bad> {
        let s = self.take(8, None)?;
        Ok(i64::from_le_bytes(s.try_into().unwrap()))
    }
    /// FName: 8 bytes on disk, 12 in memory (kis.py name)
    fn name(&mut self) -> Result<String, Bad> {
        let i = self.i32()?;
        let n = self.i32()?;
        self.mi += 4;
        let v = self.pk.names.get(i as usize).ok_or(Bad)?;
        Ok(if n == 0 { v.clone() } else { format!("{v}_{}", n - 1) })
    }
    /// object pointer: 4 bytes on disk, 8 in memory (kis.py pidx)
    fn pidx(&mut self) -> Result<Option<ObjRef>, Bad> {
        let v = self.i32()?;
        self.mi += 4;
        Ok(resolve(self.pk, v))
    }
    /// FFieldPath: count + names (+ ResolvedOwner pidx, owner = true): 8 bytes in memory (kis.py fieldpath)
    fn fieldpath(&mut self) -> Result<String, Bad> {
        let m = self.mi;
        let c = self.i32()?;
        if !(0..64).contains(&c) {
            return Err(Bad);
        }
        let mut path = Vec::new();
        for _ in 0..c {
            path.push(self.name()?);
        }
        self.i32()?;
        self.mi = m + 8;
        Ok(if path.is_empty() { "None".into() } else { path.join(".") })
    }
    fn arr(&mut self, end: u8) -> Result<Vec<Ex>, Bad> {
        let mut out = Vec::new();
        loop {
            let (_, e) = self.expr()?;
            if let Ex::Op(t) = e {
                if t == end {
                    return Ok(out);
                }
            }
            out.push(e);
            if out.len() > 100_000 {
                return Err(Bad);
            }
        }
    }
    fn bx(&mut self) -> Result<Box<Ex>, Bad> {
        Ok(Box::new(self.expr()?.1))
    }
    fn expr(&mut self) -> Result<Stmt, Bad> {
        let at = self.mi;
        let t = self.u8()?;
        let e = match t {
            0x00 | 0x01 | 0x02 | 0x48 | 0x6C => Ex::Var(t, self.fieldpath()?),
            0x04 => Ex::Return(self.bx()?),
            0x06 => Ex::Jump(self.u32()?),
            0x07 => {
                let off = self.u32()?;
                Ex::JumpIfNot(off, self.bx()?)
            }
            0x09 => {
                self.u16()?;
                self.u8()?;
                Ex::Assert(self.bx()?)
            }
            0x0B | 0x15 | 0x16 | 0x17 | 0x25 | 0x26 | 0x27 | 0x28 | 0x2A | 0x2D | 0x30 | 0x32 | 0x3A | 0x3C | 0x3E | 0x40 | 0x4A | 0x4D | 0x50 | 0x53 | 0x5A | 0x5E | 0x66 => Ex::Op(t),
            0x0C => Ex::IntConst8(self.i32()? as i64),
            0x0F => {
                let p = self.fieldpath()?;
                let v = self.bx()?;
                Ex::Let(p, v, self.bx()?)
            }
            0x14 | 0x43 | 0x44 | 0x5F | 0x60 => {
                let a = self.bx()?;
                Ex::LetKind(t, a, self.bx()?)
            }
            0x12 | 0x19 | 0x1A => {
                let o = self.bx()?;
                let off = self.u32()?;
                let rv = self.fieldpath()?;
                Ex::Context(t, o, off, rv, self.bx()?)
            }
            0x11 => {
                let p = self.fieldpath()?;
                Ex::BitFieldConst(p, self.u8()?)
            }
            0x13 | 0x2E | 0x52 | 0x54 | 0x55 => {
                let c = self.pidx()?;
                Ex::Cast(t, c, self.bx()?)
            }
            0x18 => {
                let off = self.u32()?;
                Ex::Skip(off, self.bx()?)
            }
            0x1B | 0x45 => {
                let n = self.name()?;
                Ex::Virtual(t, n, self.arr(0x16)?)
            }
            0x1C | 0x46 | 0x68 => {
                let f = self.pidx()?;
                Ex::Final(t, f, self.arr(0x16)?)
            }
            0x1D => Ex::Int(self.i32()? as i64),
            0x1E => Ex::Float(self.f32()?),
            0x1F => {
                let rest = &self.b[self.o..];
                let e = rest.iter().position(|&c| c == 0).ok_or(Bad)?;
                let v: String = rest[..e].iter().map(|&c| c as char).collect();
                self.take(e + 1, None)?;
                Ex::Str(v)
            }
            0x20 => Ex::Obj(self.pidx()?),
            0x21 => Ex::Name(self.name()?),
            0x22 | 0x23 | 0x41 => Ex::Vec3(t, [self.f32()?, self.f32()?, self.f32()?]),
            0x24 => Ex::Byte(self.u8()?),
            0x2C => Ex::Int(self.u8()? as i64),
            0x29 => {
                let lt = self.u8()?;
                Ex::Text(match lt {
                    0 => TextLit::Empty,
                    1 => {
                        let a = self.bx()?;
                        let b = self.bx()?;
                        TextLit::Localized(a, b, self.bx()?)
                    }
                    2 => TextLit::Invariant(self.bx()?),
                    3 => TextLit::Literal(self.bx()?),
                    4 => {
                        self.pidx()?;
                        let a = self.bx()?;
                        TextLit::StringTable(a, self.bx()?)
                    }
                    _ => return Err(Bad),
                })
            }
            0x2B => {
                let mut v = [0.0; 10];
                for x in v.iter_mut() {
                    *x = self.f32()?;
                }
                Ex::Transform(v)
            }
            0x2F => {
                let st = self.pidx()?;
                self.i32()?;
                Ex::StructConst(st, self.arr(0x30)?)
            }
            0x31 => {
                let a = self.bx()?;
                Ex::SetArray(a, self.arr(0x32)?)
            }
            0x33 => Ex::PropertyConst(self.fieldpath()?),
            0x34 => {
                let rest = &self.b[self.o..];
                let mut e = 0;
                while e + 1 < rest.len() && !(rest[e] == 0 && rest[e + 1] == 0) {
                    e += 2;
                }
                if e + 1 >= rest.len() {
                    return Err(Bad);
                }
                let u: Vec<u16> = rest[..e].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                self.take(e + 2, None)?;
                Ex::Str(String::from_utf16_lossy(&u))
            }
            0x35 => Ex::Int64(self.i64()?),
            0x36 => Ex::UInt64(self.i64()? as u64),
            0x37 => {
                let s = self.take(8, None)?;
                Ex::Double(f64::from_le_bytes(s.try_into().unwrap()))
            }
            0x38 => {
                let c = self.u8()?;
                Ex::PrimitiveCast(c, self.bx()?)
            }
            0x39 | 0x3B => {
                let a = self.bx()?;
                self.i32()?;
                Ex::SetColl(t, a, self.arr(if t == 0x39 { 0x3A } else { 0x3C })?)
            }
            0x3D | 0x3F | 0x65 => {
                let p = self.fieldpath()?;
                self.i32()?;
                let end = match t {
                    0x3D => 0x3E,
                    0x3F => 0x40,
                    _ => 0x66,
                };
                Ex::ConstColl(t, p, self.arr(end)?)
            }
            0x42 => {
                let p = self.fieldpath()?;
                Ex::StructMember(p, self.bx()?)
            }
            0x4B => Ex::InstanceDelegate(self.name()?),
            0x4C => Ex::PushFlow(self.u32()?),
            0x4E => Ex::ComputedJump(self.bx()?),
            0x4F => Ex::PopFlowIfNot(self.bx()?),
            0x51 => Ex::InterfaceContext(self.bx()?),
            0x5B => Ex::SkipOffsetConst(self.u32()?),
            0x5C | 0x62 => {
                let a = self.bx()?;
                Ex::MultiOp(t, a, self.bx()?)
            }
            0x5D => Ex::ClearMulticast(self.bx()?),
            0x61 => {
                let n = self.name()?;
                let a = self.bx()?;
                Ex::BindDelegate(n, a, self.bx()?)
            }
            0x63 => {
                let f = self.pidx()?;
                let d = self.bx()?;
                Ex::CallMulticast(f, d, self.arr(0x16)?)
            }
            0x64 => {
                let p = self.fieldpath()?;
                Ex::LetPersistent(p, self.bx()?)
            }
            0x67 => Ex::SoftObjectConst(self.bx()?),
            0x69 => {
                let n = self.u16()?;
                let end = self.u32()?;
                let idx = self.bx()?;
                let mut cs = Vec::new();
                for _ in 0..n {
                    let v = self.expr()?.1;
                    self.u32()?;
                    cs.push((v, self.expr()?.1));
                }
                Ex::Switch(end, idx, cs, self.bx()?)
            }
            0x6A => {
                let et = self.u8()?;
                if et == 4 {
                    self.name()?;
                }
                Ex::Instrumentation
            }
            0x6B => {
                let a = self.bx()?;
                Ex::ArrayGetByRef(a, self.bx()?)
            }
            0x6D => Ex::FieldPathConst(self.bx()?),
            _ => return Err(Bad),
        };
        Ok((at, e))
    }
}

/// Decode a whole script (statements until the bytes run out); Err when any token is malformed
fn decode(pk: &Package, b: &[u8]) -> Result<Vec<Stmt>, Bad> {
    let mut k = K { pk, b, o: 0, mi: 0 };
    let mut out = Vec::new();
    while k.o < b.len() {
        out.push(k.expr()?);
    }
    Ok(out)
}

/// FField / FProperty (UE 4.26 FProperty::Serialize + overrides): `ty` already read
fn field(pk: &Package, r: &mut mh_pak::buf::Cursor, ty: &str, depth: usize) -> Option<Prop> {
    if depth > 8 {
        return None;
    }
    let name = pk.fname(r);
    r.skip(4); // FlagsPrivate (EObjectFlags)
    r.skip(4 + 4); // ArrayDim, ElementSize
    let flags = r.u64();
    r.skip(2); // RepIndex
    pk.fname(r); // RepNotifyFunc
    r.skip(1); // BlueprintReplicationCondition
    let mut p = Prop { name, ty: ty.to_string(), flags, sub: None, inner: vec![] };
    let single = |r: &mut mh_pak::buf::Cursor| -> Option<Option<Prop>> {
        let t = pk.fname(r);
        if t == "None" {
            return Some(None);
        }
        Some(Some(field(pk, r, &t, depth + 1)?))
    };
    match ty {
        "BoolProperty" => r.skip(6), // FieldSize, ByteOffset, ByteMask, FieldMask, BoolSize, bIsNativeBool
        "ByteProperty" | "ObjectProperty" | "WeakObjectProperty" | "LazyObjectProperty" | "SoftObjectProperty" | "InterfaceProperty" | "StructProperty" | "DelegateProperty" | "MulticastDelegateProperty" | "MulticastInlineDelegateProperty" | "MulticastSparseDelegateProperty" => {
            p.sub = resolve(pk, r.s32());
        }
        "ClassProperty" | "SoftClassProperty" => {
            p.sub = resolve(pk, r.s32());
            let meta = resolve(pk, r.s32());
            if meta.is_some() {
                p.sub = meta;
            }
        }
        "EnumProperty" => {
            p.sub = resolve(pk, r.s32());
            if let Some(u) = single(r)? {
                p.inner.push(u);
            }
        }
        "ArrayProperty" | "SetProperty" => {
            if let Some(u) = single(r)? {
                p.inner.push(u);
            }
        }
        "MapProperty" => {
            for _ in 0..2 {
                if let Some(u) = single(r)? {
                    p.inner.push(u);
                }
            }
        }
        "FieldPathProperty" => {
            pk.fname(r);
        }
        _ => {}
    }
    if r.bad {
        return None;
    }
    Some(p)
}

/// A UStruct export's body after its tagged properties: (SuperStruct, ChildProperties, script bytes)
pub fn struct_body(rd: &Reader, pk: &Rc<Package>, ei: usize) -> Option<(Option<ObjRef>, Vec<Prop>, Vec<u8>, usize)> {
    let ex = pk.exports.get(ei)?;
    let mut r = pk.cursor();
    let end = ex.off + ex.size;
    r.p = ex.off;
    rd.tagged(pk, &mut r, end);
    // UObject::Serialize: non-CDO objects then carry bool HasGuid (+ FGuid) (CUE4Parse UObject.Deserialize)
    if ex.flags & mh_pak::asset::RF_CLASS_DEFAULT_OBJECT == 0 {
        let has = r.s32();
        if has != 0 {
            r.skip(16);
        }
    }
    let sup = resolve(pk, r.s32());
    let nc = r.s32();
    if !(0..100_000).contains(&nc) {
        return None;
    }
    r.skip(4 * nc as i64); // Children
    let np = r.s32();
    if !(0..100_000).contains(&np) {
        return None;
    }
    let mut props = Vec::with_capacity(np as usize);
    for _ in 0..np {
        let ty = pk.fname(&mut r);
        props.push(field(pk, &mut r, &ty, 0)?);
    }
    let _mem = r.s32(); // ScriptBytecodeSize (in-memory)
    let disk = r.s32(); // ScriptStorageSize
    if disk < 0 || r.p + disk as i64 > end || r.bad {
        return None;
    }
    let code = r.at(r.p, disk as usize)?.into_owned();
    r.p += disk as i64;
    let after = (end - r.p).max(0) as usize;
    Some((sup, props, code, after))
}

/// Every Function export whose outer is export `class_ei` (the generated class), decoded
pub fn class_functions(rd: &Reader, pk: &Rc<Package>, class_ei: usize) -> HashMap<String, Rc<Func>> {
    let mut out = HashMap::new();
    for (i, ex) in pk.exports.iter().enumerate() {
        if ex.outer != class_ei as i32 + 1 {
            continue;
        }
        if resolve(pk, ex.cls).map(|c| c.name).as_deref() != Some("Function") {
            continue;
        }
        let Some((sup, props, code, after)) = struct_body(rd, pk, i) else { continue };
        // UFunction::Serialize after the struct: FunctionFlags u32 (+ RepOffset u16 for FUNC_Net) + EventGraphFunction
        // + EventGraphCallOffset
        let flags = {
            let mut r = pk.cursor();
            r.p = ex.off + ex.size - after as i64;
            if after >= 4 {
                r.u32()
            } else {
                0
            }
        };
        let Ok(stmts) = decode(pk, &code) else { continue };
        let index = stmts.iter().enumerate().map(|(k, s)| (s.0, k)).collect();
        out.insert(ex.name.clone(), Rc::new(Func { name: ex.name.clone(), flags, props, code: stmts, index, sup }));
    }
    out
}

/// The generated class export of a Blueprint package ("<name>_C", class WidgetBlueprintGeneratedClass /
/// BlueprintGeneratedClass), with its ChildProperties (member variables) and SuperStruct
pub fn class_export(rd: &Reader, pk: &Rc<Package>) -> Option<(usize, Option<ObjRef>, Vec<Prop>)> {
    let i = pk.exports.iter().position(|e| {
        let c = resolve(pk, e.cls).map(|c| c.name).unwrap_or_default();
        c.ends_with("BlueprintGeneratedClass") && e.name.ends_with("_C")
    })?;
    let (sup, props, _, _) = struct_body(rd, pk, i).unwrap_or((None, vec![], vec![], 0));
    let sup = sup.or_else(|| resolve(pk, pk.exports[i].sup));
    Some((i, sup, props))
}

/// UserDefinedStruct field names in declaration order (for EX_StructConst of a Blueprint struct)
pub fn user_struct_fields(rd: &Reader, pkg: &str) -> Option<Vec<String>> {
    let pk = rd.open(pkg)?;
    let i = pk.exports.iter().position(|e| resolve(&pk, e.cls).map(|c| c.name).as_deref() == Some("UserDefinedStruct"))?;
    let (_, props, _, _) = struct_body(rd, &pk, i)?;
    Some(props.into_iter().map(|p| p.name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BP_NavButton's Update decodes with the same in-memory indices as state/ui_kismet/BP_NavButton.txt
    /// (scripts/ui_kismet.py): "@15 CallFunc_GetActiveWidgetIndex_ReturnValue = SubNavSwitcher.GetActiveWidgetIndex()",
    /// "@1780 NavButton.SetStyle(...)"
    #[test]
    fn decodes_nav_button() {
        let v = mh_pak::Vfs::mount_default().expect("paks");
        let rd = Reader::new(std::sync::Arc::new(v));
        let pk = rd.open("Mordhau/Content/Mordhau/UI/Shared/BP_NavButton").unwrap();
        let (ci, sup, members) = class_export(&rd, &pk).unwrap();
        assert_eq!(sup.unwrap().name, "UserWidget");
        assert!(members.iter().any(|p| p.name == "NavText"), "{:?}", members.iter().map(|p| &p.name).collect::<Vec<_>>());
        let f = class_functions(&rd, &pk, ci);
        let u = &f["Update"];
        assert!(u.index.contains_key(&15) && u.index.contains_key(&1780));
        let (at, e) = &u.code[u.index[&1780]];
        assert_eq!(*at, 1780);
        match e {
            Ex::Context(_, _, _, _, inner) => match &**inner {
                Ex::Virtual(_, n, _) | Ex::Final(_, Some(ObjRef { name: n, .. }), _) => assert_eq!(n, "SetStyle"),
                x => panic!("{x:?}"),
            },
            x => panic!("{x:?}"),
        }
        let ug = &f["ExecuteUbergraph_BP_NavButton"];
        assert_eq!(ug.params().next().unwrap().name, "EntryPoint");
        assert!(ug.index.contains_key(&82));
    }
}
