/* mordhau_core.h - C ABI of mordhau-core-ffi (hand-written; keep in sync with src/lib.rs).
 * One world per thread. Strings are NUL-terminated UTF-8. Inputs are applied on the next mh_world_step.
 * Spec JSON: the RecordsJson format (core/tests/golden/spec.json, written by godot/tools/export_golden.gd from the
 * user's own game install; never ship it). */
#ifndef MORDHAU_CORE_H
#define MORDHAU_CORE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct MhWorld MhWorld;

typedef struct MhFighterState {
    int32_t motion_kind;          /* 0 Idle 1 Attack 2 Parry 3 Feinted 4 Blocked 5 Flinch 6 Stun 7 Disarmed */
    int32_t stamina;
    int32_t health;
    int32_t dead;
    double start_time;
    double end_time;
    int32_t attack_stage;         /* EAttackStage (attack) / EParryStage (parry), -1 otherwise */
    int32_t attack_move;          /* EAttackMove, -1 when not attacking */
    int32_t attack_type;          /* EAttackType, -1 when not attacking */
    int32_t movement_restriction; /* EMovementRestriction */
    int32_t has_weapon;
} MhFighterState;

enum {
    MH_INPUT_ATTACK = 0,        /* a = EAttackMove (0 RightStrike 1 LeftStrike 2 Stab 3 AltStab 4 Kick), angle deg */
    MH_INPUT_FEINT = 1,
    MH_INPUT_PARRY = 2,         /* a = EBlockType (0 Regular 1 AltRegular 2 ShieldWall); block pressed */
    MH_INPUT_RELEASE_BLOCK = 3,
    MH_INPUT_SWITCH_MODE = 4,
    MH_INPUT_TOGGLE_MODE = 5
};

MhWorld *mh_world_new(const char *spec_json, double dt);          /* NULL when the spec does not parse */
MhWorld *mh_world_new_from_install(const char *weapons_json, double dt); /* spec matrix + paks; weapons_json = ["<Blueprint path>", ...]; NULL on failure, see mh_last_error */
const char *mh_last_error(void);                                   /* last failure message of this thread, "" if none; valid until the next call */
void mh_world_free(MhWorld *w);
int32_t mh_world_add_fighter(MhWorld *w, const char *name, const char *weapon_path, const char *left_path); /* index or -1 */
int32_t mh_world_push_input(MhWorld *w, int32_t fighter, int32_t kind, int32_t a, double angle);            /* 0 / -1 */
int32_t mh_world_push_contact(MhWorld *w, int32_t attacker, int32_t defender, const char *bone);          /* bone NULL = Spine1 */
int32_t mh_world_set_stamina(MhWorld *w, int32_t fighter, int32_t v);
void mh_world_step(MhWorld *w);
double mh_world_now(MhWorld *w);
int32_t mh_fighter_state(MhWorld *w, int32_t fighter, MhFighterState *out);
/* JSON getters: write when cap > needed (NUL-terminated) and return the length needed without the NUL */
size_t mh_world_state_json(MhWorld *w, char *buf, size_t cap);
size_t mh_world_event_count(MhWorld *w);
size_t mh_world_event_json(MhWorld *w, size_t i, char *buf, size_t cap);

#ifdef __cplusplus
}
#endif
#endif
