//! Exe-exact combat (the default, `World::new` / `Precision::Exe`) vs the GDScript reference (`World::new_reference`,
//! feature `reference_compat`, bit-exact with the golden traces). One code path; `World::qf()` rounds each operation in
//! Exe mode. Every difference, with its source:
//!
//!  1. Arithmetic: every float field of UMordhauMotion and its subclasses, FAttackInfo, the stat components and the
//!     character is a binary32 (PDB layouts, extract/native/types/*.h: `float`), and every operation on them is an SSE
//!     ss op (addss / subss / mulss / divss, no FMA, no x87): e.g. UAttackMotion::OnBegin_Implementation rva=0x162eda0
//!     WindupEnd = Windup + StartTime, UMordhauMotion::Tick rva=0x166ee80 `EndTime <= TimeSeconds` on floats. Exe
//!     mode rounds each result to f32 (exact: f64 holds any f32 sum / product / quotient's correctly-rounded value
//!     before the second rounding). The reference computes in f64 and rounds only where it emulates the exe
//!     (AssignNetAttackMotion's lag, PackedFloat32Array damage, Vector2 records).
//!  2. Records: the exe holds every UE float as its f32 (the cooked package stores 4-byte floats; native ctor stores
//!     are 32-bit immediates). `RecordsJsonExe` rounds the reference's record dump to f32, which is the mh-spec matrix's
//!     value (data_gen/spec stores each UE float as its f32); the reference keeps the f64 of the package's JSON decimal
//!     (0.56 vs 0.560000002384185791015625f).
//!  3. Clock: UWorld TimeSeconds (+0x598, float) += DeltaSeconds (float) once per frame before the tick groups (the
//!     frame order mh-character's exe.rs documents, step 0). So `now` after n steps of 1/120 s is the f32 running sum,
//!     not n * dt; e.g. 0.8999955 instead of 0.9 at tick 450 of 2 ms (golden `shield_wall_kite`: a contact scheduled
//!     exactly at StartTime + ShieldWallRaiseTime is not yet parried by the shield wall in the exe; tests/exe.rs).
//!     The reference uses n * dt in f64.
//!  4. UMordhauMotion::Tick rva=0x166ee80 (decomp `EndTime < TimeSeconds || EndTime == TimeSeconds` -> OnEnded;
//!     byte-matched src/Mordhau/Private/Motions/MordhauMotion.cpp) has no current-motion test: when OnTick already
//!     switched the motion, OnEnded still runs on the old one (UAttackMotion::OnEnded_Implementation rva=0x1631260 and
//!     UBlockedMotion::OnEnded_Implementation rva=0x1663650 then still fire a queued move; the base and
//!     UFeintedMotion re-check). The reference adds `sys.motion == self`.
//!  5. UIdleMotion::OnBegin_Implementation rva=0x1660d30 stores FLT_MAX exactly (0x7f7fffff, byte-matched
//!     src/.../IdleMotion.cpp); the reference's literal parses to FLT_MAX + 1 ulp in Godot.
//!  6. Stamina byte: UStaminaStatComponent::SetStatValue_Internal rva=0x1515c80 (decomp 204-227) maps in float and
//!     rounds with RoundToInt ((int)ROUND(2x + 0.5) >> 1); the reference rounds a double half away from zero. Same
//!     result for every integer stamina in [0, 100] (tests/exe.rs exe_stamina_byte_round_to_int).
//!  7. GetSmoothedWindUpNormalizedTime rva=0x1628bf0 evaluates OffsetTerm / ReleaseTerm / the exponent in float and
//!     calls powf (FMath::Pow; byte-matched C in src/.../AttackMotion.cpp, NONMATCHING only in register choice);
//!     the reference uses f64 pow.
//!  9. Tracers: UAttackMotion::OnBegin_Implementation rva=0x162eda0 calls AMordhauWeapon::OnAttackStarted
//!     (native body rva=0x162eb40, decomp UAttackMotion.cpp 2240), which calls ResetTracers (vcall +0x7d8,
//!     rva=0x163bb80: bAreCurrentTracersInvalidated = 1), so a new attack's first PrepareForTracing is invalid and the
//!     first swept segment starts one late tick later; the reference never resets, sweeping from wherever the weapon
//!     was at the end of the previous attack. (OnAttackStarted also enables the ClashCollider: see 14.)
//!  8. Input angles (AMordhauCharacter::RequestAttack float Angle) and the attack-angle clamp / PackFloat run in float.
//! 10. Multi-hit tracing: ExecuteAttackTracingAndLogic rva=0x161c390 stops a swept segment only when
//!     ProcessHitForDamage returns false or the motion changed; ProcessHitForDamage's tail (CanContinueTracingAfter-
//!     DealingDamage -> AssignNetMotionDynamicParam(| 1)) and its early outs return true, so one release can damage
//!     several characters. ProcessHitForBlocking false moves on to the next hit. The reference stops after the first
//!     damage (melee_hit.rs process_damage returns !exe).
//! 11. UAttackMotion::OnLeave_Implementation rva=0x16323e0: leaving Release without a hit for an attack (miss-combo)
//!     costs -(int)MissStaminaCost.
//! 12. Same function: leaving Recovery for a parry with no hit, bCanCombo and bUseSeamlessCFTPInRecovery costs
//!     -FeintCost; the BlockCollider and ClashCollider are disabled (DisableBlockCollider rva=0x1538cc0).
//! 13. Component hit pipeline (when the host gives FighterGeom; mh-sim does): a trace hit is a body, the defender's
//!     BlockCollider or a weapon ClashCollider (HitComp). Implemented:
//!     - ProcessHitForBlocking rva=0x1638380.
//!     - CheckParry rva=0x164ed40: a BlockCollider hit records the attack in BlockedAttacks. A timed parry also needs
//!       TestForwardParry rva=0x166dbd0, which passes Direction = Start - End to FMath::LineBoxIntersection
//!       0x140e5e9a0, so only segments starting inside the forward box pass. Body / weapon hits are parried only from
//!       BlockedAttacks within Timed/HeldBlockMemoryDuration, plus the CheckSimpleBlockDirectional rva=0x164f3b0
//!       angle tests (MaxParryAngle on camera - attacker root, MaxParryWeaponAngle on the trace direction).
//!     - CheckAttackParry rva=0x1616cd0, CheckChamber rva=0x1617140 and CheckClash rva=0x16179a0 with the same shapes
//!       (geometry.rs).
//!     The reference parries a body hit by timing alone.
//! 14. Collider enable flags: the BlockCollider is enabled by UAttackMotion / UParryMotion OnBegin (UParryMotion
//!     OnLeave rva=0x1665b10 keeps it for a flinch). The ClashCollider is enabled by OnAttackStarted rva=0x162eb40.
//!     The BlockCollider's world transform is AMordhauCharacter::UpdateBlockCollider rva=0x15700a0
//!     (geometry::update_block_collider: Original/Low/High offsets by ComputeParryHeight rva=0x164f930, under the 1P
//!     camera). UNCONFIRMED: the ParryWeapon transform it multiplies in, and the ClashCollider shape (native
//!     component, no template).
//!
//! Checked equal to the reference (byte-matched C in src/Mordhau/Private, read for this list): UAttackMotion
//! IsInLiveRecovery, GetMovementRestriction, IsInEarlyRelease, OnEnded, CanMorphInto, ConvertToCombo (NONMATCHING,
//! same logic); UParryMotion CanInitiateMotion, ProcessBlock, ProcessFeint, GetMovementRestriction, OnLeave
//! (EnterParryRecovery(TotalBlocks == 0)), StopHolding; UStabMotion::ModifyAttackInfo; UKickMotion
//! GetMovementRestriction; UFeintedMotion::ProcessFeint; UStatComponent StopRegeneration / SetStatValue_Internal /
//! OffsetStatValue; UStaminaStatComponent::OffsetStamina (plus an AMordhauGameMode bDisableStamina early-out the
//! reference lacks: no ported game mode sets it, UNCONFIRMED for custom modes).
//!
//! Not exe-exact yet (UNCONFIRMED): the tracer in this crate keeps the reference's Godot-space metres (mh-sim's
//! `trace` module is the UE-space exe tracer); FRichCurve evaluation runs in f64 then rounds the result (the engine
//! evaluates in float; curve values are only read by animation-facing fields and LastReleaseNormalizedTime).
