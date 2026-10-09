"""Only synthetic stored-property fixtures; no original capsule values."""
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('shadow_cache',Path(__file__).with_name('shadow_cache.py'))
s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)

def fixture():
    return [{'Type':'PhysicsAsset','Properties':{'SkeletalBodySetups':[{'ObjectPath':s.PACKAGE+'.1'}]}},
            {'Type':'SkeletalBodySetup','Properties':{'BoneName':'synthetic_bone','AggGeom':{'SphylElems':[
              {'Center':dict(X=1,Y=2,Z=3),'Rotation':dict(Pitch=4,Yaw=5,Roll=6),'Radius':7,'Length':8}]}}}]

class Tests(unittest.TestCase):
    def test_exact_stored_components_and_order(self):
        row=s.capsules(fixture())['capsules'][0]
        self.assertEqual(row,dict(bone='synthetic_bone',center=[1,2,3],rotation_pyr=[4,5,6],radius=7,length=8))
    def test_missing_geometry_is_rejected(self):
        e=fixture();del e[1]['Properties']['AggGeom']['SphylElems'][0]['Radius']
        with self.assertRaises(ValueError):s.capsules(e)
    def test_refs_wrong_package_or_export_and_duplicates_reject(self):
        for path in ['foreign.1',s.PACKAGE+'.0',s.PACKAGE+'.-1',s.PACKAGE+'.999999999']:
            e=fixture();e[0]['Properties']['SkeletalBodySetups'][0]['ObjectPath']=path
            with self.assertRaises(ValueError):s.capsules(e)
        e=fixture();e[0]['Properties']['SkeletalBodySetups']*=2
        with self.assertRaises(ValueError):s.capsules(e)
    def test_nonfinite_or_boolean_geometry_rejects(self):
        for value in [float('nan'),float('inf'),True,0,-1]:
            e=fixture();e[1]['Properties']['AggGeom']['SphylElems'][0]['Radius']=value
            with self.assertRaises(ValueError):s.capsules(e)

if __name__=='__main__':unittest.main()
