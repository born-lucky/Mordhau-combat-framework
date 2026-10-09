# bt_switch_equipment.gd - UBTTask_SwitchEquipment, ported from extract/native/decomp/UBTTask_SwitchEquipment.cpp
#   PerformSwitchingTask rva=0x1497830, ctor rva=0x144dcb0 (bMelee +0x70 = 1, AllowedSubclasses +0x78,
#   NotAllowedSubclasses +0x88 empty). Node values from the BehaviorTree package (extract/json).
# Walks the inventory (AMordhauCharacter +0x11e8): the first item that qualifies is either already in a hand
# (Succeeded) or gets an equip motion (InProgress); none qualifies -> Failed. While an equip motion runs -> InProgress.
#
# Ported for melee (bMelee): bCanAttack (+0xcb9) and an AMordhauWeapon. The ranged branch (bMelee false) also needs
# bAllowFire (+0xcbc), a virtual (vtable +0x6f0) and an ammo/projectile class (+0x648) that are not ported:
# UNCONFIRMED, such an item is treated as qualifying only if `ranged` is set on it. Allowed/NotAllowed lists are
# matched by class name: the json gives the UClass path, inventory entries carry their weapon class path.
# The equip motion (FNetMotion MotionType 8, slot index) is not in the combat port: requested as an event.
class_name BtSwitchEquipment
extends BtTask

const FN := "UBTTask_SwitchEquipment::PerformSwitchingTask rva=0x1497830"

func execute(c) -> int:
	return perform(c)

func tick(c, _dt: float) -> int:
	return perform(c)

func _matches(item: Dictionary, names: PackedStringArray) -> bool:
	var w: WeaponData = item.get("weapon")
	var ids := PackedStringArray([w.id if w != null else ""])
	if w != null:
		ids.append_array(w.chain)
		ids.append(w.native_class)
	if item.get("is_fists", false):
		ids.append("FistsWeapon")
	for n in names:
		for i in ids:
			if String(i).get_file() == n:
				return true
	return false

func perform(c) -> int:
	var me: BotBody = c.body
	if me == null or me.sys == null:
		return FAILED
	if me.sys.motion != null and me.sys.motion.kind().begins_with("Equip"):	# UEquipMotion StaticClass 0x14168f900
		return IN_PROGRESS
	var melee: bool = params.b_melee
	var allowed := params.allowed_subclasses
	var denied := params.not_allowed_subclasses
	for i in me.inventory.size():
		var item: Dictionary = me.inventory[i]
		var w: WeaponData = item.get("weapon")
		if w == null:
			continue
		var ok: bool = (w.b_can_attack) if melee else bool(item.get("ranged", false))
		if not ok:
			continue
		if not allowed.is_empty() and not _matches(item, allowed):
			continue
		if not denied.is_empty() and _matches(item, denied):
			continue
		if i == me.right_hand:								# RightHandEquipment +0x11f8 / LeftHandEquipment +0x1200
			c.note("SwitchEquipment", "%s already in hand -> Succeeded" % w.id, FN)
			return SUCCEEDED
		c.note("SwitchEquipment", "equip slot %d (%s) -> InProgress" % [i, w.id], FN)
		c.events.append({"t": c.now(), "kind": "equip", "slot": i})	# AssignNetMotion {MotionType 8, slot}
		return IN_PROGRESS
	return FAILED
