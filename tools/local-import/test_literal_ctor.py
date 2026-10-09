"""Synthetic x64 bytes only, with independent expected object/array words."""
import importlib.util,struct,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('literal_ctor',Path(__file__).with_name('literal_ctor.py'))
l=importlib.util.module_from_spec(spec);import sys;sys.modules[spec.name]=l;spec.loader.exec_module(l)
class PE:
 base=0x140000000
 def __init__(self,code,data=b''):self.code=code;self.data=data
 def read_rva(self,rva,size):
  if 0x1000<=rva<0x1000+len(self.code):return self.code[rva-0x1000:rva-0x1000+size]
  if 0x2000<=rva<0x2000+len(self.data):return self.data[rva-0x2000:rva-0x2000+size]
  raise ValueError('range')
def proc(name,at,size):return dict(name=name+'::'+name,rva=at,size=size)
class Tests(unittest.TestCase):
 def test_scalar_alias_and_original_rip_vector_stores(self):
  # mov rbx,rcx / xor eax,eax / mov[rbx],eax / mov[rbx+4],11223344 / movups xmm0,[rip+data] / movups[rbx+8],xmm0 / ret
  code=b'\x48\x89\xcb\x31\xc0\x89\x03\xc7\x43\x04\x44\x33\x22\x11'
  address=0x1000+len(code);code+=b'\x0f\x10\x05'+struct.pack('<i',0x2000-address-7)+b'\x0f\x11\x43\x08\xc3'
  data=bytes(range(16));r=l.Replay(PE(code,data),[proc('FSynthetic',0x1000,len(code))],24).apply(0x1000)
  self.assertEqual(r['bytes'],b'\0'*4+b'\x44\x33\x22\x11'+data);self.assertEqual(r['unsupported'],[])
 def test_lexical_array_header_not_heap_index(self):
  # array header at object+8; mov rax,[rcx+8], xor ebp,ebp; literal[rax+rbp*4]; inc-like add; second literal;ret
  code=bytes.fromhex('488b410831edc704a80100000083c501c704a802000000c3')
  r=l.Replay(PE(code),[proc('FArrayFixture',0x1000,len(code))],32,[8]).apply(0x1000)
  self.assertEqual(r['arrays'],{8:[1,2]});self.assertEqual(r['bytes'],bytes(32))
 def test_nested_constructor_offset_and_unknown_clobber(self):
  # Preserve object alias in rbx; RCX points at subobject; direct nested call; then unknown call + unknown EAX store.
  first=bytes.fromhex('4889cb488d4908')
  first+=b'\xe8'+struct.pack('<i',0x1040-(0x1000+len(first)+5))
  first+=b'\xe8'+struct.pack('<i',0x3000-(0x1000+len(first)+5))+bytes.fromhex('894310c3')
  child=bytes.fromhex('c70178563412c3');code=first+bytes(0x40-len(first))+child
  r=l.Replay(PE(code),[proc('FOuter',0x1000,len(first)),proc('FInner',0x1040,len(child))],24).apply(0x1000)
  self.assertEqual(r['bytes'][8:12],bytes.fromhex('78563412'));self.assertEqual(r['bytes'][16:20],bytes(4))
  self.assertTrue(any('Unknown object store' in x['reason'] for x in r['unsupported']))
 def test_folded_names_keep_exact_body_and_conflicting_ranges_reject(self):
  code=bytes.fromhex('c70178563412c3')
  rows=[proc('FOne',0x1000,len(code)),proc('FTwo',0x1000,len(code))]
  r=l.Replay(PE(code),rows,4).apply(0x1000)
  self.assertEqual(r['bytes'],bytes.fromhex('78563412'))
  self.assertEqual(r['calls'][0]['aliases'],['FOne::FOne','FTwo::FTwo'])
  rows[1]['size']+=1
  with self.assertRaises(ValueError):l.Replay(PE(code),rows,4).apply(0x1000)
 def test_sse_unpack_shuffle_high_low_routes_use_independent_lanes(self):
  data=struct.pack('<8I',1,2,3,4,11,12,13,14)
  for operation,expected in [('0f14c1',(1,11,2,12)),('0f15c1',(3,13,4,14)),('0f12c1',(13,14,3,4)),('0f16c1',(1,2,11,12)),('0fc6c14e',(3,4,11,12))]:
   code=bytearray()
   for prefix,rva in [(bytes.fromhex('0f1005'),0x2000),(bytes.fromhex('0f100d'),0x2010)]:
    address=0x1000+len(code);code.extend(prefix+struct.pack('<i',rva-address-len(prefix)-4))
   code.extend(bytes.fromhex(operation+'0f1101c3'))
   r=l.Replay(PE(bytes(code),data),[proc('FLanes',0x1000,len(code))],16).apply(0x1000)
   self.assertEqual(struct.unpack('<4I',r['bytes']),expected);self.assertEqual(r['unsupported'],[])
 def test_memory_scalar_load_zero_upper_then_unpack(self):
  data=struct.pack('<ff',1.25,-2.5);code=bytearray()
  for prefix,rva in [(bytes.fromhex('f30f1005'),0x2000),(bytes.fromhex('f30f100d'),0x2004)]:
   address=0x1000+len(code);code.extend(prefix+struct.pack('<i',rva-address-len(prefix)-4))
  code.extend(bytes.fromhex('0f14c10f1101c3'))
  r=l.Replay(PE(bytes(code),data),[proc('FScalarLanes',0x1000,len(code))],16).apply(0x1000)
  self.assertEqual(struct.unpack('<4f',r['bytes']),(1.25,-2.5,0,0))
 def test_unknown_upper_lanes_are_not_invented(self):
  prefix=bytes.fromhex('0f100d');code=prefix+struct.pack('<i',0x2000-0x1000-len(prefix)-4)+bytes.fromhex('f30f10c10f1101c3')
  r=l.Replay(PE(code,bytes(range(16))),[proc('FUnknownLanes',0x1000,len(code))],16).apply(0x1000)
  self.assertEqual(r['bytes'],bytes(16));self.assertTrue(any('Unknown object store' in x['reason'] for x in r['unsupported']))
 def test_overlapping_stack_scalar_write_invalidates_old_vector(self):
  # Spill all16bytes, overwrite its second lane, then reload16. The exact-sized
  # stack cache must refuse that wider load rather than reproduce stale bytes.
  prefix=bytes.fromhex('0f1005');code=prefix+struct.pack('<i',0x2000-0x1000-len(prefix)-4)
  code+=bytes.fromhex('0f114424e0c74424e4123456780f104c24e00f1109c3')
  r=l.Replay(PE(code,bytes(range(16))),[proc('FStackOverlap',0x1000,len(code))],16).apply(0x1000)
  self.assertEqual(r['bytes'],bytes(16))
  self.assertTrue(any('Unknown object store' in x['reason'] for x in r['unsupported']))
 def test_packed_object_load_retains_actual_byte_lanes(self):
  # Four object words →MOVUPS→SHUFPS→object. Expected words are independent.
  code=bytes.fromhex('c70101000000c7410402000000c7410803000000c7410c040000000f10010fc6c04e0f114110c3')
  r=l.Replay(PE(code),[proc('FObjectLanes',0x1000,len(code))],32).apply(0x1000)
  self.assertEqual(struct.unpack('<8I',r['bytes']),(1,2,3,4,3,4,1,2))
  self.assertEqual(r['unsupported'],[])
 def test_movsd_register_preserves_destination_high_lanes(self):
  data=struct.pack('<8I',1,2,3,4,11,12,13,14);code=bytearray()
  for prefix,rva in [(bytes.fromhex('0f1005'),0x2000),(bytes.fromhex('0f100d'),0x2010)]:
   address=0x1000+len(code);code.extend(prefix+struct.pack('<i',rva-address-len(prefix)-4))
  code.extend(bytes.fromhex('f20f10c10f1101c3'))
  r=l.Replay(PE(bytes(code),data),[proc('FMovsdRegister',0x1000,len(code))],16).apply(0x1000)
  self.assertEqual(struct.unpack('<4I',r['bytes']),(11,12,3,4));self.assertEqual(r['unsupported'],[])
 def test_known_partial_upper_prefix_survives_scalar_register_move(self):
  data=struct.pack('<8I',1,2,3,4,11,12,13,14);code=bytearray()
  for prefix,rva in [(bytes.fromhex('0f100d'),0x2000),(bytes.fromhex('0f1015'),0x2010)]:
   address=0x1000+len(code);code.extend(prefix+struct.pack('<i',rva-address-len(prefix)-4))
  # First MOVSD has an unknown destination: only low8 known. MOVSS must
  # preserve its known second lane, while the remaining high8 stay unknown.
  code.extend(bytes.fromhex('f20f10c1f30f10c2660fd601c3'))
  r=l.Replay(PE(bytes(code),data),[proc('FPartialPrefix',0x1000,len(code))],8).apply(0x1000)
  self.assertEqual(struct.unpack('<2I',r['bytes']),(11,2));self.assertEqual(r['unsupported'],[])
 def test_packed_bitwise_masks_use_exact_bytes(self):
  data=struct.pack('<8I',0xabcdef01,0x12345678,0x7fffffff,0x80000000,0xff00ff00,0x00ff00ff,0x80000000,0x00000001)
  for opcode,expected in [('0f54c1',(0xab00ef00,0x00340078,0,0)),('0f56c1',(0xffcdff01,0x12ff56ff,0xffffffff,0x80000001)),('0f57c1',(0x54cd1001,0x12cb5687,0xffffffff,0x80000001))]:
   code=bytearray()
   for prefix,rva in [(bytes.fromhex('0f1005'),0x2000),(bytes.fromhex('0f100d'),0x2010)]:
    address=0x1000+len(code);code.extend(prefix+struct.pack('<i',rva-address-len(prefix)-4))
   code.extend(bytes.fromhex(opcode+'0f1101c3'))
   r=l.Replay(PE(bytes(code),data),[proc('FBitwise',0x1000,len(code))],16).apply(0x1000)
   self.assertEqual(struct.unpack('<4I',r['bytes']),expected);self.assertEqual(r['unsupported'],[])
 def partial_source(self):
  data=struct.pack('<8I',1,2,3,4,11,12,13,14);code=bytearray()
  for prefix,rva in [(bytes.fromhex('0f1005'),0x2000),(bytes.fromhex('0f100d'),0x2010)]:
   address=0x1000+len(code);code.extend(prefix+struct.pack('<i',rva-address-len(prefix)-4))
  code.extend(bytes.fromhex('f30f10d1')) # XMM2 unknown high12; only source low4=[11] known.
  return code,data
 def test_partial_source_movsd_and_movq_do_not_shift_upper_into_gap(self):
  for operation in ['f20f10c2','f30f7ec2']:
   code,data=self.partial_source();code.extend(bytes.fromhex(operation+'660fd601c3'))
   r=l.Replay(PE(bytes(code),data),[proc('FPartialSource',0x1000,len(code))],8).apply(0x1000)
   self.assertEqual(r['bytes'],bytes(8))
   self.assertTrue(any('Unknown object store' in x['reason'] for x in r['unsupported']))
 def test_partial_source_unpack_knows_only_two_valid_output_lanes(self):
  code,data=self.partial_source()
  code.extend(bytes.fromhex('f20f10c20f14c10f1101660fd64110c3'))
  r=l.Replay(PE(bytes(code),data),[proc('FPartialSourceUnpack',0x1000,len(code))],24).apply(0x1000)
  self.assertEqual(struct.unpack('<6I',r['bytes']),(0,0,0,0,11,11))
  self.assertTrue(any('Unknown object store' in x['reason'] for x in r['unsupported']))
if __name__=='__main__':unittest.main()
