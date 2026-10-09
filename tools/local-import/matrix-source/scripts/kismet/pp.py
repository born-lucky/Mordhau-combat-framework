from kis import *
def P(e):
    t=e[0]; a=e[1:-1]
    if t in (0,1,2,0x48): return a[0]
    if t==0x6c: return a[0]
    if t==0x04: return 'return '+P(a[0])
    if t==0x06: return 'goto %d'%a[0]
    if t==0x07: return 'if !(%s) goto %d'%(P(a[1]),a[0])
    if t==0x0B: return 'nop'
    if t in (0x0F,): return '%s = %s'%(P(a[1]),P(a[2]))
    if t in (0x14,0x43,0x44,0x5F,0x60): return '%s = %s'%(P(a[0]),P(a[1]))
    if t in (0x12,0x19,0x1A): return '%s.%s'%(P(a[0]),P(a[3]))
    if t in (0x13,0x2E,0x52,0x54,0x55): return 'Cast<%s>(%s)'%(a[0],P(a[1]))
    if t==0x18: return P(a[1])
    if t in (0x1B,0x45): return '%s(%s)'%(a[0],', '.join(P(x) for x in a[1]))
    if t in (0x1C,0x46,0x68): return '%s(%s)'%(a[0],', '.join(P(x) for x in a[1]))
    if t in (0x1D,0x1E,0x24,0x2C,0x35,0x36,0x37): return repr(a[0])
    if t==0x1F or t==0x34: return '"%s"'%a[0]
    if t==0x20: return 'obj:'+a[0]
    if t==0x21: return "N'%s'"%a[0]
    if t in (0x22,0x23,0x41): return 'V%r'%(a[0],)
    if t==0x25: return '0'
    if t==0x26: return '1'
    if t==0x27: return 'true'
    if t==0x28: return 'false'
    if t==0x2A: return 'null'
    if t==0x17: return 'self'
    if t==0x29: return 'text'
    if t==0x2F: return '%s{%s}'%(a[0],', '.join(P(x) for x in a[1]))
    if t==0x31: return '%s = [%s]'%(P(a[0]),', '.join(P(x) for x in a[1]))
    if t==0x38: return 'cast%d(%s)'%(a[0],P(a[1]))
    if t==0x42: return '%s.%s'%(P(a[1]),a[0])
    if t==0x4B: return 'delegate:'+a[0]
    if t==0x4C: return 'push %d'%a[0]
    if t==0x4D: return 'pop'
    if t==0x4E: return 'goto [%s]'%P(a[0])
    if t==0x4F: return 'if !(%s) pop'%P(a[0])
    if t==0x53: return 'end'
    if t==0x5B: return 'skipoff %d'%a[0]
    if t==0x63: return '%s.Broadcast(%s)'%(P(a[0]),', '.join(P(x) for x in a[1]))
    if t==0x64: return '%s = %s'%(a[0],P(a[1]))
    if t==0x67: return P(a[0])
    if t==0x69: return 'switch(%s){%s; default %s}'%(P(a[1]),'; '.join('%s:%s'%(P(c[0]),P(c[1])) for c in a[2]),P(a[3]))
    if t==0x6B: return '%s[%s]'%(P(a[0]),P(a[1]))
    if t==0x6A: return 'instr'
    return 'tok%x%r'%(t,a)
if __name__ == '__main__':
    # python scripts/kismet/pp.py <raw dir from `sh scripts/mdx.sh raw`> <package path without extension> <out.txt>
    #   -> every Function export's bytecode as pseudo-code lines "<in-memory statement index>  <statement>"
    #   (the index is the one EX_Jump / ubergraph entry points use).
    import sys, os
    raw, pkgp, out = sys.argv[1], sys.argv[2], sys.argv[3]
    p = Pkg(os.path.join(raw, pkgp + '.uasset'), os.path.join(raw, pkgp + '.uexp'))
    with open(out, 'w') as f:
        for e in p.exports:
            if p.cls_name(e) != 'Function':
                continue
            f.write('// %s\n' % e['name'])
            for st in script(p, e):
                f.write('%6d  %s\n' % (st[-1], P(st)))
