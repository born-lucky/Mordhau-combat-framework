"""Synthetic consumer contracts: no original generated records are fixtures."""
import copy
import importlib.util
from pathlib import Path
import re
import unittest

spec=importlib.util.spec_from_file_location('weapon_records_under_test',Path(__file__).with_name('weapon_records.py'))
w=importlib.util.module_from_spec(spec);spec.loader.exec_module(w)


def accepted_leaves(value,path=''):
    if isinstance(value,dict) and value:
        out=[]
        for key,child in value.items():out.extend(accepted_leaves(child,path+'.'+key if path else key))
        return out
    return [{'field':path,'accepted':True}]


def native_documents(classes):
    # Identifiers are fixture labels, deliberately unrelated to supported hashes.
    identity={'schema_version':1,'exe_sha1':'synthetic-executable','pdb_sha1':'synthetic-symbols'}
    return ({**identity,'classes':classes},
            {**identity,'classes':{k:accepted_leaves(v) for k,v in classes.items()},
             'zero_allocation':{'memset_import':'memset'}})


def synthetic_value(kind):
    return {'bool':False,'int':3,'float':1.25,'string':{},
            'vec2':{'X':2.0,'Y':5.0},'vec3':{'X':2.0,'Y':5.0,'Z':7.0},
            'float_list':[11.0,22.0,33.0,44.0],'string_list':[],'json':[]}[kind]


def complete_fixture(alternate=True):
    """Populate authored field identities/types with arbitrary synthetic values."""
    wm=w.binding(w.WEAPON_SOURCE,'MAP');am=w.binding(w.ATTACK_SOURCE,'MAP')
    wk=w.field_types(w.WEAPON_SOURCE);ak=w.field_types(w.ATTACK_SOURCE)
    attack={ue:synthetic_value(ak[field]) for ue,field in am.items()}
    weapon={ue:synthetic_value(wk[field]) for ue,field in wm.items()
            if ue not in w.OPTIONAL_CATALOG|w.BLUEPRINT_ONLY}
    for ue in w.binding(w.WEAPON_SOURCE,'ATTACKS'):
        if not ue.startswith('Base'):weapon[ue]=copy.deepcopy(attack)
    methods={'b':'bool','f':'float','v3':'vec3','rot':'vec3','obj':'string'}
    for field,method,ue in re.findall(r'^\s*(\w+) = r\.(b|f|v3|rot|obj)\("(\w+)"\)',w.EQUIP_SOURCE.read_text(),re.M):
        if ue=='bAllowShieldWall':continue
        weapon[ue]=synthetic_value(methods[method])
    normal='Synthetic/NormalProfile';alt='Synthetic/AlternateProfile';path='Synthetic/BP_TestWeapon'
    weapon['WeaponAnimationProfileClass']={'ObjectPath':normal+'.12'}
    weapon['SecondWeaponAnimationProfileClass']={'ObjectPath':alt+'.19'}
    weapon['bHasAlternateMode']=alternate
    classes={'AMordhauWeapon':weapon,'FAttackInfo':attack}
    data,review=native_documents(classes)
    row={'native_root':'MordhauWeapon','serialized_defaults':{
            'EquipmentName':{'LocalizedString':'Synthetic fixture'},
            'StabAttack':{'Windup':0.375,'Damage':[9.0,8.0,7.0,6.0]},
            'SecondStabAttack':{'Release':0.625}},
         'chain_child_first':[path],'original_packages':[]}
    packages={path:row}
    for profile,stab,side in [(normal,'Synthetic/NormalStab','Synthetic/OppositeStab'),
                              (alt,'Synthetic/AltGripStab','Synthetic/AltGripOppositeStab')]:
        packages[profile]={'serialized_defaults':{},'maps_merged':{'Attacks':{
            'EAttackMove::Stab':{'ObjectPath':stab+'.4'},
            'EAttackMove::AltStab':{'ObjectPath':side+'.5'}}}}
        packages[stab]={'serialized_defaults':{}};packages[side]={'serialized_defaults':{}}
    return {'records':packages,'weapons':[path]},data,review


class NativeReviewTests(unittest.TestCase):
    def test_nested_leaf_needs_acceptance_and_empty_values_also_need_coverage(self):
        data,review=native_documents({'AMock':{'TurnCaps':{'X':2.0,'Y':5.0},'NullCurve':{},'Names':[]}})
        w.validate_native(data,review)
        for field in ('TurnCaps.Y','NullCurve','Names'):
            broken=copy.deepcopy(review)
            broken['classes']['AMock']=[r for r in broken['classes']['AMock'] if r['field']!=field]
            with self.subTest(field=field),self.assertRaisesRegex(ValueError,'Unreviewed native field'):
                w.validate_native(data,broken)
    def test_rejected_or_conflicting_leaf_cannot_supply_a_default(self):
        data,review=native_documents({'AMock':{'Speed':3.0}})
        for rows in [[{'field':'Speed','accepted':False}],
                     [{'field':'Speed','accepted':True},{'field':'Speed','accepted':False}]]:
            broken=copy.deepcopy(review);broken['classes']['AMock']=rows
            with self.subTest(rows=rows),self.assertRaisesRegex(ValueError,'Unreviewed native field'):
                w.validate_native(data,broken)
    def test_original_identities_and_zero_fill_proof_must_match(self):
        data,review=native_documents({'AMock':{'Speed':3.0}})
        for key in ('exe_sha1','pdb_sha1'):
            broken=copy.deepcopy(review);broken[key]='another-original'
            with self.subTest(key=key),self.assertRaisesRegex(ValueError,'identity mismatch'):
                w.validate_native(data,broken)
        for proof in ({},{'memset_import':'other-function'}):
            broken=copy.deepcopy(review);broken['zero_allocation']=proof
            with self.subTest(proof=proof),self.assertRaisesRegex(ValueError,'allocation proof unavailable'):
                w.validate_native(data,broken)


class AttackTests(unittest.TestCase):
    def setUp(self):
        self.maps={'Damage':'damage','HeadBonus':'head_bonus','LegBonus':'leg_bonus','Windup':'windup','TurnCaps':'turn_caps'}
        self.kinds={'damage':'float_list','head_bonus':'float_list','leg_bonus':'float_list','windup':'float','turn_caps':'vec2'}
        self.native={'Damage':[10.0,20.0,30.0,40.0],'HeadBonus':[1.0]*4,'LegBonus':[-1.0]*4,
                     'Windup':0.875,'TurnCaps':{'X':50.0,'Y':80.0}}
    def test_blueprint_replaces_arrays_and_overrides_only_written_nested_fields(self):
        original=copy.deepcopy(self.native)
        result=w.attack_record(self.native,{'Damage':[4.0,3.0,2.0,1.0],'Windup':0.375,'TurnCaps':{'Y':120.0}},self.maps,self.kinds,'fixture')
        self.assertEqual(result['damage'],[4.0,3.0,2.0,1.0]);self.assertEqual(result['turn_caps'],[50.0,120.0])
        self.assertEqual(result['windup'],0.375);self.assertEqual(result['head_bonus'],[1.0]*4)
        self.assertEqual(self.native,original)
    def test_missing_native_or_wrong_armor_cardinality_never_falls_back(self):
        missing=copy.deepcopy(self.native);del missing['Windup']
        with self.assertRaisesRegex(ValueError,'original attack field unavailable'):
            w.attack_record(missing,{},self.maps,self.kinds,'fixture')
        for field in ('Damage','HeadBonus','LegBonus'):
            for count in (0,3,5):
                with self.subTest(field=field,count=count),self.assertRaisesRegex(ValueError,'four original armor-tier values'):
                    w.attack_record(self.native,{field:[1.0]*count},self.maps,self.kinds,'fixture')
    def test_nonfinite_or_bool_numeric_values_fail_in_scalar_vector_and_array(self):
        for bad in (float('nan'),float('inf'),-float('inf'),True,'1.0'):
            for delta in ({'Windup':bad},{'TurnCaps':{'Y':bad}},{'Damage':[1.0,2.0,bad,4.0]}):
                with self.subTest(bad=bad,delta=delta),self.assertRaisesRegex(ValueError,'finite number'):
                    w.attack_record(self.native,delta,self.maps,self.kinds,'fixture')


class WeaponTests(unittest.TestCase):
    def test_stab_side_and_alternate_grip_are_distinct_maps_and_attack_structs(self):
        packages,data,review=complete_fixture()
        records,unknown=w.build_records(packages,data,review);result=records['BP_TestWeapon']
        self.assertEqual(result['motions']['STAB'],'Synthetic/NormalStab')
        self.assertEqual(result['motions']['ALT_STAB'],'Synthetic/OppositeStab')
        self.assertEqual(result['alt_motions']['STAB'],'Synthetic/AltGripStab')
        self.assertEqual(result['alt_motions']['ALT_STAB'],'Synthetic/AltGripOppositeStab')
        self.assertEqual(result['weapon']['stab']['windup'],0.375)
        self.assertEqual(result['weapon']['second_stab']['release'],0.625)
        self.assertEqual(result['weapon']['second_stab']['windup'],1.25)
        self.assertEqual(result['equip']['weapon_animation_profile'],'Synthetic/NormalProfile')
        self.assertEqual(result['equip']['second_weapon_animation_profile'],'Synthetic/AlternateProfile')
        self.assertEqual(unknown,[])
    def test_normal_grip_only_does_not_publish_alternate_mode(self):
        packages,data,review=complete_fixture(alternate=False)
        records,_=w.build_records(packages,data,review)
        self.assertNotIn('alt_motions',records['BP_TestWeapon'])
    def test_opaque_catalog_label_is_reported_unknown_not_invented_empty_text(self):
        packages,data,review=complete_fixture()
        del packages['records']['Synthetic/BP_TestWeapon']['serialized_defaults']['EquipmentName']
        records,unknown=w.build_records(packages,data,review)
        self.assertNotIn('display_name',records['BP_TestWeapon']['weapon'])
        self.assertEqual(len(unknown),1);self.assertEqual(unknown[0]['field'],'EquipmentName')
    def test_original_scan_errors_and_native_seeded_packages_are_rejected(self):
        packages,data,review=complete_fixture()
        errored=copy.deepcopy(packages);errored['errors']=['fixture scan failed']
        with self.assertRaisesRegex(ValueError,'scan reported errors'):w.build_records(errored,data,review)
        for key,value in [('native_defaults_included',True),('decode_errors',['fixture decode failed'])]:
            broken=copy.deepcopy(packages);broken['records']['Synthetic/BP_TestWeapon'][key]=value
            with self.subTest(key=key),self.assertRaisesRegex(ValueError,'Invalid original-only package'):
                w.build_records(broken,data,review)
    def test_missing_motion_reference_cannot_become_an_empty_profile(self):
        packages,data,review=complete_fixture();del packages['records']['Synthetic/OppositeStab']
        with self.assertRaisesRegex(ValueError,'motion package unavailable'):w.build_records(packages,data,review)


if __name__=='__main__':unittest.main()
