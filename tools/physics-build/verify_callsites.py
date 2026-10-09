"""Check MSVC emitted direct virtual-call offsets, not just header declarations."""
import re

# Facts from the installed x64 interface. Offsets are bytes in the vtable.
EXPECTED = {
    "mh_abi_scene_release": 0x08, "mh_abi_scene_add": 0x50,
    "mh_abi_scene_filter": 0x188, "mh_abi_scene_simulate": 0x1c0,
    "mh_abi_scene_fetch": 0x1e8, "mh_abi_cook_triangle": 0x28,
    "mh_abi_cook_convex": 0x40, "mh_abi_physics_scene": 0x90,
    "mh_abi_physics_static": 0xa8, "mh_abi_physics_body": 0xb0,
    "mh_abi_physics_constraint": 0xf0, "mh_abi_physics_material": 0x100,
    "mh_abi_physics_triangle": 0x28, "mh_abi_physics_convex": 0x58,
    "mh_abi_body_pose": 0xa0, "mh_abi_body_mass": 0x100,
    "mh_abi_body_inertia": 0x118, "mh_abi_body_mass_set": 0xf8,
    "mh_abi_body_inertia_set": 0x110, "mh_abi_body_com": 0xe8,
    "mh_abi_body_attach": 0xb8, "mh_abi_shape_local": 0x88,
    "mh_abi_shape_filter": 0x98,
}


def check(text):
    results = []
    for name, expected in EXPECTED.items():
        blocks = re.findall(r"^" + name + r"\s+PROC\b[^\n]*\n(.*?)^" + name + r"\s+ENDP\b", text, re.MULTILINE | re.DOTALL)
        if len(blocks) != 1:
            raise ValueError("Missing/nonunique compiler probe: " + name)
        offsets = []
        # MSVC labels REX-prefixed indirect tail jumps rex_jmp in /FAs output.
        # This is the same vtable dispatch, not an alternate expected slot.
        for match in re.finditer(r"^\s*(?:call|jmp|rex_jmp)\s+QWORD PTR\s+\[\w+\s*(?:\+\s*([0-9A-Fa-f]+)(h)?)?\]", blocks[0], re.MULTILINE):
            number, hexadecimal = match.groups()
            offsets.append(int(number, 16 if hexadecimal else 10) if number else 0)
        if offsets != [expected]:
            raise ValueError(f"Unexpected emitted virtual offset {name}: {offsets}, expected {expected}")
        results.append(dict(symbol=name, expected_indirect_offset=expected, observed_indirect_offset=offsets[0]))
    return results
