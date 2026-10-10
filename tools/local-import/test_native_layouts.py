"""Synthetic layout descriptors and independent scalar/bit/array values."""
import importlib.util,struct,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('native_layouts',Path(__file__).with_name('native_layouts.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
class Tpi:
 complete={'FInner':100,'UBase':101,'UOuter':102}
 def udt(self,ti):return None,{100:'FInner',101:'UBase',102:'UOuter'}[ti]
class Types:
 def __init__(self,t):pass
 def decl(self,ti):return {1:'float',2:'int32',3:'TArray<float>',4:'FInner',5:'UObject *',6:'bool',7:'uint8'}[ti]
 def size(self,ti):return {1:4,2:4,3:16,4:20,5:8,6:1,7:1}[ti]
 def layout(self,ti):
  field=lambda name,off,t:dict(name=name,off=off,ti=t)
  return {
   100:dict(size=20,bases=[],members=[field('InnerFloat',0,1),field('Array',4,3)]),
   101:dict(size=4,bases=[],members=[field('BaseInt',0,2)]),
   102:dict(size=40,bases=[dict(name='UBase',off=0)],members=[field('Inner',4,4),field('Object',24,5),field('Enabled',32,6),field('Packed',33,7)])}[ti]
class CV:
 Types=Types
 @staticmethod
 def bitinfo(t,ti):return (7,3,2) if ti==7 else None
class Tests(unittest.TestCase):
 def test_reviewed_unknown_bytes_omit_values_instead_of_zero_substitution(self):
  d=m.Layouts(CV,Tpi());raw=bytearray(40);struct.pack_into('<i',raw,0,-12)
  known=bytearray([1])*40;known[4:8]=bytes(4)
  values,review=d.decode_reviewed('UOuter',raw,{},known,[])
  self.assertEqual(values['BaseInt'],-12);self.assertNotIn('InnerFloat',values['Inner'])
  row=next(v for v in review if v['field']=='Inner.InnerFloat')
  self.assertFalse(row['accepted']);self.assertIn('unknown',row['reason'])
  self.assertEqual(values['Object'],{});self.assertTrue(next(v for v in review if v['field']=='Object')['accepted'])
 def test_reviewed_nonnull_pointer_never_acquires_null_default(self):
  d=m.Layouts(CV,Tpi());raw=bytearray(40);raw[24]=1
  values,review=d.decode_reviewed('UOuter',raw,{},bytes([1])*40,[])
  self.assertNotIn('Object',values)
  self.assertIn('Non-null',next(v for v in review if v['field']=='Object')['reason'])
 def test_reviewed_array_checks_original_count_and_conditional_elements(self):
  d=m.Layouts(CV,Tpi());raw=bytearray(40);struct.pack_into('<i',raw,16,1)
  words=[int.from_bytes(struct.pack('<f',3.5),'little')]
  values,_=d.decode_reviewed('UOuter',raw,{8:words},bytes([1])*40,[{'array_offset':8,'raw':'00006040','conditional':False}])
  self.assertEqual(values['Inner']['Array'],[3.5])
  values,_=d.decode_reviewed('UOuter',raw,{8:words},bytes([1])*40,[{'array_offset':8,'raw':'00006040','conditional':True}])
  self.assertNotIn('Array',values['Inner'])
 def test_exact_base_nested_binary32_array_and_bit_offsets(self):
  d=m.Layouts(CV,Tpi());raw=bytearray(40);struct.pack_into('<i',raw,0,-12);struct.pack_into('<f',raw,4,1.25);raw[32]=1;raw[33]=0b10100
  words=[int.from_bytes(struct.pack('<f',v),'little') for v in (2.5,-3.75)]
  self.assertEqual(d.array_offsets('UOuter'),{8})
  self.assertEqual(d.decode('UOuter',raw,{8:words}),dict(BaseInt=-12,Inner=dict(InnerFloat=1.25,Array=[2.5,-3.75]),Enabled=True,Packed=5))
  self.assertEqual(d.skipped_fields[0]['field'],'Object');self.assertEqual(d.skipped_fields[0]['object_offset'],24)
 def test_truncated_object_and_unknown_identity_reject(self):
  d=m.Layouts(CV,Tpi())
  with self.assertRaises(ValueError):d.decode('UOuter',bytes(8),{})
  with self.assertRaises(ValueError):d.layout('Missing')
 def test_opaque_text_name_omitted_but_required_empty_value_wrapper_kept(self):
  class OpaqueTpi:
   complete={'UOpaqueFixture':100,'FText':101,'FName':102,'FPerspective':103}
   def udt(self,ti):return None,{100:'UOpaqueFixture',101:'FText',102:'FName',103:'FPerspective'}[ti]
  class OpaqueTypes:
   def __init__(self,t):pass
   def decl(self,ti):return {1:'FText',2:'FName',3:'FPerspective'}[ti]
   def size(self,ti):return 8
   def layout(self,ti):
    if ti==100:return dict(size=24,bases=[],members=[dict(name='EquipmentName',off=0,ti=1),dict(name='BoneName',off=8,ti=2),dict(name='RequiredWrapper',off=16,ti=3)])
    return dict(size=8,bases=[],members=[])
  class OpaqueCV:
   Types=OpaqueTypes
   @staticmethod
   def bitinfo(t,ti):return None
  layouts=m.Layouts(OpaqueCV,OpaqueTpi())
  self.assertEqual(layouts.decode('UOpaqueFixture',bytes(24),{}),{'RequiredWrapper':{}})
  self.assertEqual([(v['field'],v['type'],v['object_offset']) for v in layouts.skipped_fields], [('EquipmentName','FText',0),('BoneName','FName',8)])
  self.assertTrue(all('Opaque runtime' in v['reason'] for v in layouts.skipped_fields))
if __name__=='__main__':unittest.main()
