// DecompGame - label the image from our PDB-derived tables, then decompile only Mordhau's own functions.
// Run headless after `-import ... -noanalysis` (full auto-analysis + 1 GB PDB does not fit in commit limit).
// args: <labels.tsv> <game_functions.tsv> <out dir> [timeout s, default 60] [mode: all|prepare|decomp]
//       [data=<labels_data.tsv>] [locals=<locals.tsv>]
// prepare = labels + functions only (so ApplyTypes.java can type them); decomp = skip labelling, decompile.
// labels.tsv: .text offset \t qualified name   game_functions.tsv: name section offset size obj (header row)
// labels_data.tsv (scripts/data_labels.py): va(hex) \t type \t name \t source   -> labels for .rdata/.data globals
// locals.tsv (scripts/native_locals.py): the PDB's S_LOCAL + S_DEFRANGE_* records per game function
//
// Readability passes (round 3), each measured by scripts/readability.py:
//  prepare: (a) labels for global data, so `_DAT_1457e9af0` reads `GUObjectArray`;
//           (b) a Function at every PDB-labelled call target of a game function. The decompiler names a callee
//               only from a Function symbol: a bare label still prints func_0x<addr>. The new function's signature
//               stays SourceType.DEFAULT (unlocked), so the decompiler still infers arguments at each call site.
//  decomp:  (c) linker ICF (/OPT:ICF): one body per address, printed once, under the name whose class matches the
//               typed `this` (else the first PDB name); the other names point to it.
//           (d) PDB local names: each decompiler variable (HighSymbol) is matched to the S_LOCAL whose def-range
//               holds the same storage (register bytes, or frame slot) at the PC where the variable is written.
//           (e) virtual calls: `(**(code **)(*(longlong *)X + 0x158))(X)` / `(*(code *)...vfptr[0xe].Foo)(this)`
//               become `X->Name(...)` when X's static type has a <Class>_vtbl (made by ApplyTypes.java) and the
//               call's first argument is that same object.
// Round 6 ("a wrong name is worse than none", measured against the round-5 output):
//  (d') unique-space instances are checked in the machine storage they provably occupy (uniqueStorage), else the
//       variable is not named; a stack-slot variable must be the whole local (its size); no definition may assign a
//       value whose own type (valueType: call returns, typed members, parameters) contradicts the PDB type.
//  (e'') a virtual call's class comes from the PDB local live in the register the vtable pointer was loaded through
//       (pdbObjectClass), else from the value's own type; never from the merged variable's printed type. Slots past
//       that class's vftable are named only when every class deriving from it agrees (vtbl_types.json "ext").
//  (f)  variables whose printed class-pointer type contradicts a value they are assigned are retyped to the common
//       base of their values (retypeConflicts).
//  Side tables: _local_names.tsv and _vcalls.tsv, both re-checked by scripts/check_local_names.py.
//  The C text is printed from the decompiler's own token tree (PrettyPrinter.getLines) with (d) and (e) applied as
//  per-token overrides. With no overrides the printer must reproduce DecompiledFunction.getC() byte for byte; it
//  is checked per function and falls back to getC() on any mismatch.
//@category Mordhau
import ghidra.app.script.GhidraScript;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.cmd.function.CreateFunctionCmd;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.data.*;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.pcode.*;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.*;
import ghidra.util.StringUtilities;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.regex.*;
import com.google.gson.*;

public class DecompGame extends GhidraScript {
	Address text;
	DataTypeManager dtm;
	CategoryPath pdbCat = new CategoryPath("/MordhauPDB");

	Address at(long off) { return text.add(off); }
	long toff(Address a) { return a.getOffset() - text.getOffset(); }   // .text section offset, as in the PDB

	// Windows caps a path component at 255 chars; long template owners get a stable hash suffix.
	String fname(String k) { return k.length() <= 120 ? k : k.substring(0, 100) + "_" + Integer.toHexString(k.hashCode()); }

	// File a function belongs to. MSVC names nested code `Outer::Fn'::`N'::... (lambdas, EH unwind funclets dtor$N,
	// static-init thunks); those go with Outer's class so a class file holds everything compiled from its methods.
	String owner(String n) {
		if (n.startsWith("`")) {
			int q = n.indexOf('\'');
			String in = q > 1 ? n.substring(1, q) : n.substring(1);
			if (in.startsWith("dynamic initializer") || in.startsWith("dynamic atexit") || in.startsWith("`dynamic")) return "_static_init";
			n = in;
		}
		return n.contains("::") ? n.substring(0, n.lastIndexOf("::")) : "_global";
	}
	String fileOf(String n) { return fname(owner(n).replaceAll("[^A-Za-z0-9_]", "_")); }

	// compiler-generated pieces get a note so nobody mistakes them for source functions
	String kind(String n) {
		if (n.matches(".*::(dtor|catch|fin|filt)\\$\\d+$")) return " [compiler-generated exception-unwind funclet]";
		if (n.contains("dynamic initializer") || n.contains("dynamic atexit")) return " [compiler-generated static initializer]";
		return "";
	}

	String clean(String s) { return s.replaceAll("[^A-Za-z0-9_:~<>,\\-]", "_"); }

	// ------------------------------------------------------------------ prepare
	int dataLabels(Path p) throws IOException {
		SymbolTable st = currentProgram.getSymbolTable();
		int n = 0;
		try (BufferedReader r = Files.newBufferedReader(p, StandardCharsets.UTF_8)) {
			for (String l; (l = r.readLine()) != null; ) {
				String[] c = l.split("\t", -1);
				if (c.length < 3) continue;
				try {
					Address a = toAddr(Long.parseUnsignedLong(c[0], 16));
					if (!currentProgram.getMemory().contains(a)) continue;
					st.createLabel(a, clean(c[2]), SourceType.IMPORTED); n++;
				} catch (Exception e) {}
			}
		}
		return n;
	}

	// A Function at each call/jump target and each code address taken as data (LEA of a delegate target) of the game
	// functions, when a PDB label marks it as an entry point. Returns {created, already functions, skipped}.
	int[] calleeFunctions(List<String[]> rows) {
		Listing li = currentProgram.getListing();
		SymbolTable st = currentProgram.getSymbolTable();
		MemoryBlock tb = currentProgram.getMemory().getBlock(".text");
		Set<Address> want = new LinkedHashSet<>();
		for (String[] r : rows) {
			Function f = getFunctionAt(at(Long.parseLong(r[2])));
			if (f == null) continue;
			for (Instruction ins : li.getInstructions(f.getBody(), true))
				for (Reference ref : ins.getReferencesFrom()) {
					Address d = ref.getToAddress();
					if (!tb.contains(d) || f.getBody().contains(d)) continue;
					RefType t = ref.getReferenceType();
					if (t.isCall() || t.isJump() || t.isData()) want.add(d);
				}
		}
		int made = 0, had = 0, skip = 0;
		for (Address d : want) {
			if (monitor.isCancelled()) break;
			if (getFunctionAt(d) != null) { had++; continue; }
			Symbol s = st.getPrimarySymbol(d);
			if (s == null || s.getSource() != SourceType.IMPORTED) { skip++; continue; }   // only PDB entry points
			if (li.getInstructionAt(d) == null) new DisassembleCommand(d, null, true).applyTo(currentProgram, monitor);
			if (li.getInstructionAt(d) == null) { skip++; continue; }
			new CreateFunctionCmd(null, d, null, SourceType.DEFAULT).applyTo(currentProgram, monitor);
			if (getFunctionAt(d) != null) made++; else skip++;
		}
		return new int[] { made, had, skip };
	}

	// MSVC x64 C++ EH funclets (`Fn'::`1'::dtor$N unwind, catch$N catch blocks, fin$N __finally) are not called like
	// C functions: the runtime calls them through _CallSettingFrame (VC/Tools/MSVC/14.44.35207/crt/src/x64/handlers.asm,
	// lines 42-49: "mov CsFrame.Handler[rsp], rcx / mov CsFrame.Establisher[rsp], rdx / mov rdx, [rdx] ; dereference
	// establisher pointer / mov rax, rcx / ... / call rax"), so RCX = the funclet's own address and RDX = the parent
	// function's establisher frame; the funclet reaches the parent's locals at EstablisherFrame + offset. A catch
	// funclet returns the continuation address (_CallSettingFrame returns the handler's RAX). Without a prototype the
	// decompiler printed these as param_1/param_2 (92% of all param_ in round 3). Only address groups whose names are
	// all funclets of one kind get the prototype (an ICF group shared with ordinary functions keeps its own).
	int funcletPrototypes(List<String[]> rows) {
		Map<String, List<String>> grp = new LinkedHashMap<>();
		for (String[] r : rows) grp.computeIfAbsent(r[2], k -> new ArrayList<>()).add(r[0]);
		Pattern fk = Pattern.compile(".*::(dtor|catch|fin)\\$\\d+$");
		int n = 0;
		for (Map.Entry<String, List<String>> e : grp.entrySet()) {
			String kind = null; boolean ok = true;
			for (String nm : e.getValue()) {
				Matcher m = fk.matcher(nm);
				if (!m.matches() || (kind != null && !kind.equals(m.group(1).equals("catch") ? "catch" : "unwind"))) { ok = false; break; }
				kind = m.group(1).equals("catch") ? "catch" : "unwind";
			}
			if (!ok || kind == null) continue;
			Function f = getFunctionAt(at(Long.parseLong(e.getKey())));
			if (f == null) continue;
			try {
				List<Parameter> ps = List.of(
					new ParameterImpl("HandlerAddress", dtm.getPointer(VoidDataType.dataType), currentProgram, SourceType.IMPORTED),
					new ParameterImpl("EstablisherFrame", dtm.getPointer(CharDataType.dataType), currentProgram, SourceType.IMPORTED));
				DataType rt = kind.equals("catch") ? dtm.getPointer(VoidDataType.dataType) : VoidDataType.dataType;
				f.updateFunction("__fastcall", new ReturnParameterImpl(rt, currentProgram), ps, Function.FunctionUpdateType.DYNAMIC_STORAGE_ALL_PARAMS, true, SourceType.IMPORTED);
				n++;
			} catch (Exception ex) { println("funclet prototype " + e.getValue().get(0) + ": " + ex); }
		}
		return n;
	}

	// ------------------------------------------------------------------ locals (PDB) -> decompiler variables
	static class PV {
		String name, type, inl; boolean param;
		PV parent; long poff;   // a member of the aggregate local `parent` at byte offset poff (S_DEFRANGE_SUBFIELD_REGISTER)
		List<String> regName = new ArrayList<>(); List<long[]> reg = new ArrayList<>();   // {byteoff, size, start, len}
		List<long[]> regGaps = new ArrayList<>(), frGaps = new ArrayList<>();   // per range: {gapoff, gaplen, ...} from its start
		List<String> frBase = new ArrayList<>(); List<long[]> fr = new ArrayList<>();     // {off, start(-1 = whole fn), len}
	}
	static class PF { String lfp, pfp; List<PV> vars = new ArrayList<>(); }
	Map<Long, PF> pdbLocals = new HashMap<>();
	boolean debugNames = System.getenv("DECOMP_DEBUG") != null;

	static long[] gaps(String[] q, int from) {   // trailing "off:len" gap fields of a locals.tsv range
		long[] g = new long[Math.max(0, q.length - from) * 2];
		for (int i = from; i < q.length; i++) { String[] x = q[i].split(":"); g[2 * (i - from)] = Long.parseLong(x[0]); g[2 * (i - from) + 1] = Long.parseLong(x[1]); }
		return g;
	}

	void loadLocals(Path p) throws IOException {
		PF cur = null;
		try (BufferedReader r = Files.newBufferedReader(p, StandardCharsets.UTF_8)) {
			for (String l; (l = r.readLine()) != null; ) {
				String[] c = l.split("\t", -1);
				if (c[0].equals("F")) {
					cur = new PF(); cur.lfp = c[3]; cur.pfp = c[4];
					if (pdbLocals.putIfAbsent(Long.parseLong(c[1]), cur) != null) cur = null;   // ICF: first record wins
				}
				else if (c[0].equals("V") && cur != null && c.length >= 6) {
					PV v = new PV(); v.name = c[1]; v.type = c[2]; v.param = c[3].equals("P"); v.inl = c[4];
					Map<Long, PV> subs = new TreeMap<>();
					for (String rg : c[5].split(";")) {
						String[] q = rg.split(",");
						if (q[0].equals("S")) {   // S,reg,byteoff,size,offset in parent,start,len
							long po = Long.parseLong(q[4]);
							PV ch = subs.get(po);
							if (ch == null) { ch = new PV(); ch.name = v.name; ch.type = ""; ch.param = v.param; ch.inl = v.inl; ch.parent = v; ch.poff = po; subs.put(po, ch); }
							ch.regName.add(q[1]); ch.reg.add(new long[] { Long.parseLong(q[2]), Long.parseLong(q[3]), Long.parseLong(q[5]), Long.parseLong(q[6]) }); ch.regGaps.add(gaps(q, 7));
							continue;
						}
						if (q[0].equals("R")) { v.regName.add(q[1]); v.reg.add(new long[] { Long.parseLong(q[2]), Long.parseLong(q[3]), Long.parseLong(q[4]), Long.parseLong(q[5]) }); v.regGaps.add(gaps(q, 6)); }
						else if (q[0].equals("F")) { v.frBase.add(v.param ? cur.pfp : cur.lfp); v.fr.add(new long[] { Long.parseLong(q[1]), Long.parseLong(q[2]), Long.parseLong(q[3]) }); v.frGaps.add(gaps(q, 4)); }
						else if (q[0].equals("B")) { v.frBase.add(q[1]); v.fr.add(new long[] { Long.parseLong(q[2]), Long.parseLong(q[3]), Long.parseLong(q[4]) }); v.frGaps.add(gaps(q, 5)); }
					}
					if (!v.name.isEmpty() && !v.name.startsWith("$")) { cur.vars.add(v); cur.vars.addAll(subs.values()); }
				}
			}
		}
	}

	// Ghidra stack offsets (0 = return address at entry) of RSP after the prologue and of RBP, by walking the
	// prologue: PUSH r (+8), SUB RSP,imm, MOV EAX,imm + CALL __chkstk + SUB RSP,RAX, LEA RBP,[RSP+d] / MOV RBP,RSP.
	long[] frame(Function f) {
		long depth = 0, chk = 0; Long rbp = null;
		Instruction ins = getInstructionAt(f.getEntryPoint());
		for (int i = 0; ins != null && i < 32; i++, ins = ins.getNext()) {
			String m = ins.getMnemonicString();
			Register r0 = ins.getRegister(0), r1 = ins.getRegister(1);
			String r0n = r0 == null ? "" : r0.getName(), r1n = r1 == null ? "" : r1.getName();
			if (m.equals("PUSH")) depth += 8;
			else if (m.equals("SUB") && r0n.equals("RSP")) {
				Scalar s = ins.getScalar(1);
				if (s != null) depth += s.getValue();
				else if (r1n.equals("RAX")) depth += chk;
			}
			else if (m.equals("MOV") && r0n.equals("EAX") && ins.getScalar(1) != null) chk = ins.getScalar(1).getValue();
			else if (m.equals("MOV") && r0n.equals("RBP") && r1n.equals("RSP")) rbp = -depth;
			else if (m.equals("LEA") && r0n.equals("RBP")) {
				long dd = 0; boolean rsp = false;
				for (Object x : ins.getOpObjects(1)) {
					if (x instanceof Register rr && rr.getName().equals("RSP")) rsp = true;
					if (x instanceof Scalar sc) dd = sc.getSignedValue();
				}
				if (rsp) rbp = -depth + dd;
			}
			else if (m.equals("CALL")) { if (chk == 0) break; }
			else if (m.startsWith("J") || m.equals("RET")) break;
		}
		return new long[] { -depth, rbp == null ? Long.MIN_VALUE : rbp };
	}

	static final Pattern AUTO = Pattern.compile("(?:this_\\d+|[a-z]*[A-Z]?Var\\d+|local_[0-9a-f]+(?:_\\d+)?|[a-zA-Z]*StackX?_[0-9a-f]+|param_\\d+)");
	static final Set<String> RESERVED = new HashSet<>(Arrays.asList("this", "int", "float", "char", "bool", "void", "long",
		"short", "double", "return", "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue",
		"goto", "true", "false", "code", "undefined", "struct", "union", "new", "delete", "class", "operator", "auto"));

	// member path of `type` at byte offset off, through base subobjects (not named) and nested structs:
	// {"Location_X", leaf DataType}, or null when the type is unknown or the offset is in no named member
	Object[] member(String type, long off) {
		String tn = type.replace("const ", "").replace("volatile ", "").trim();
		DataType t = dtm.getDataType(pdbCat, dtName(tn));
		List<String> path = new ArrayList<>();
		for (int i = 0; i < 16; i++) {
			if (t instanceof TypeDef td) t = td.getBaseDataType();
			if (!(t instanceof Structure st)) break;
			DataTypeComponent c = st.getComponentContaining((int) off);
			if (c == null || c.getFieldName() == null || c.getDataType() instanceof Undefined || c.getDataType() instanceof DefaultDataType) return null;
			if (!c.getFieldName().startsWith("super_")) path.add(c.getFieldName());
			off -= c.getOffset(); t = c.getDataType();
			if (off == 0 && !(t instanceof Structure)) break;
		}
		if (path.isEmpty() || off != 0) return null;
		return new Object[] { String.join("_", path), t };
	}

	static boolean inR(long pc, long s, long n) { return pc >= s && pc < s + n; }

	// Type sanity between the PDB variable (printed type) and the decompiler's variable: same category (F float,
	// I integer/bool/enum, P pointer/reference, S struct by value; '?' = undefined, matches anything), and for
	// pointers to two known classes, one must be a base of the other (super_ chain of the ApplyTypes layouts).
	static char catOfPdb(String t) {
		t = t.replace("const ", "").replace("volatile ", "").trim();
		if (t.endsWith("*") || t.endsWith("&")) return 'P';
		if (t.equals("float") || t.equals("double")) return 'F';
		if (t.matches("(unsigned )?(bool|char|wchar_t|char16_t|short|int|long|__int64|int8|uint8|int16|uint16|int32|uint32|int64|uint64)|enum .*")) return 'I';
		return t.isEmpty() ? '?' : 'S';
	}
	static char ghCat(DataType d) {
		if (d instanceof TypeDef td) d = td.getBaseDataType();
		if (d == null || d instanceof Undefined || d instanceof DefaultDataType) return '?';
		if (d instanceof Pointer) return 'P';
		if (d instanceof AbstractFloatDataType) return 'F';
		if (d instanceof AbstractIntegerDataType || d instanceof BooleanDataType || d instanceof ghidra.program.model.data.Enum) return 'I';
		if (d instanceof Composite) return 'S';
		return '?';
	}
	boolean related(String a, String b) { return a.equals(b) || isBase(a, b, 0) || isBase(b, a, 0); }
	boolean isBase(String sub, String base, int depth) {   // is `base` a super_ component (transitively) of `sub`?
		if (depth > 24) return false;
		DataType d = dtm.getDataType(pdbCat, sub);
		if (!(d instanceof Structure st)) return false;
		for (DataTypeComponent x : st.getDefinedComponents()) {
			if (x.getFieldName() == null || !x.getFieldName().startsWith("super_")) continue;
			String n = x.getDataType().getName();
			if (n.equals(base) || isBase(n, base, depth + 1)) return true;
		}
		return false;
	}
	boolean typeOk(PV p, DataType d) {
		char a = catOfPdb(p.type), b = ghCat(d);
		if (a == '?' || b == '?') return true;
		if (a != b) return false;
		if (a == 'P' && d instanceof Pointer gp) {
			DataType pt = gp.getDataType();
			String pn = p.type.replace("const ", "").replaceAll("[*&\s]+$", "").trim();
			if (pt instanceof Structure && dtm.getDataType(pdbCat, pn) instanceof Structure) return related(pn, pt.getName());
		}
		return true;
	}

	// ---- round 6: sizes, value types, and PDB static types at a PC
	// byte size of a PDB type as locals.tsv prints it (pointer/reference 8, primitives, PDB structs by name); -1 unknown
	long pdbSize(String t) {
		t = t.replace("const ", "").replace("volatile ", "").trim();
		if (t.endsWith("*") || t.endsWith("&")) return 8;
		switch (t) {
			case "bool": case "char": case "signed char": case "unsigned char": case "int8": case "uint8": return 1;
			case "short": case "unsigned short": case "wchar_t": case "char16_t": case "int16": case "uint16": return 2;
			case "int": case "unsigned int": case "long": case "unsigned long": case "float": case "int32": case "uint32": case "char32_t": return 4;
			case "__int64": case "unsigned __int64": case "double": case "int64": case "uint64": return 8;
		}
		if (t.startsWith("enum ")) return -1;
		DataType d = dtm.getDataType(pdbCat, dtName(t));
		return d == null || d.getLength() <= 0 ? -1 : d.getLength();
	}

	// Machine storage of a decompiler instance in the unique space: {kind 0 register / 1 stack, offset, 1 if its
	// definition is only a read of that storage}. Two proven shapes:
	//  - the decompiler's own COPY of a register/stack varnode (inserted where values meet): the value IS that
	//    storage at the copy's PC (checked like a use there);
	//  - a temporary of the defining instruction's raw p-code that the instruction then copies (or zero/sign
	//    extends) into a register, e.g. MOV R14,qword ptr [RAX+0x1a0] = `$U = LOAD ...; R14 = COPY $U`: the value is
	//    in that register (its low bytes) right after the instruction (checked like a def). Exactly one such register.
	// every reader of vn is a MULTIEQUAL/INDIRECT whose output is a register/stack instance of the same variable hv
	static boolean mergeOnly(Varnode vn, HighVariable hv) {
		Iterator<PcodeOp> ds = vn.getDescendants();
		int n = 0;
		while (ds != null && ds.hasNext()) {
			PcodeOp u = ds.next();
			int oc = u.getOpcode();
			Varnode o = u.getOutput();
			if ((oc != PcodeOp.MULTIEQUAL && oc != PcodeOp.INDIRECT) || o == null || o.getHigh() != hv) return false;
			if (!o.isRegister() && !o.getAddress().isStackAddress()) return false;
			n++;
		}
		return n > 0;
	}

	long[] uniqueStorage(Varnode vn, int depth) {
		PcodeOp d = vn.getDef();
		if (d == null || depth > 8) return null;
		if (d.getOpcode() == PcodeOp.COPY) {
			Varnode in = d.getInput(0);
			if (in.isRegister()) return new long[] { 0, in.getOffset(), 1 };
			if (in.getAddress().isStackAddress()) return new long[] { 1, in.getOffset(), 1 };
			if (in.isUnique()) {
				long[] r = uniqueStorage(in, depth + 1);
				return r == null ? null : new long[] { r[0], r[1], 1 };
			}
		}
		Instruction ins = getInstructionAt(d.getSeqnum().getTarget());
		if (ins == null) return null;
		long[] hit = null;
		for (PcodeOp r : ins.getPcode()) {
			int oc = r.getOpcode();
			if (oc != PcodeOp.COPY && oc != PcodeOp.INT_ZEXT && oc != PcodeOp.INT_SEXT) continue;
			Varnode o = r.getOutput(), in = r.getInput(0);
			if (o == null || !o.isRegister() || !in.isUnique() || in.getOffset() != vn.getOffset() || in.getSize() != vn.getSize()) continue;
			if (hit != null) return null;
			hit = new long[] { 0, o.getOffset(), 0 };
		}
		return hit;
	}

	static DataType unTd(DataType d) { return d instanceof TypeDef td ? td.getBaseDataType() : d; }
	static final DataType NULLPTR = new PointerDataType(VoidDataType.dataType);   // marker: the value is the constant 0

	// The type of the value a varnode holds, from where it came from only (never from the variable it is assigned
	// to, which is what the decompiler merges and guesses): a call's PDB prototype return, a load of a typed member
	// (member path through PTRSUB/INT_ADD offsets from a base whose own value type is a pointer to that struct), the
	// function's own PDB parameters, through COPY/CAST/INDIRECT/MULTIEQUAL (all inputs related: the base-most), and
	// PTRSUB of a member/base sub-object. null = not provable; NULLPTR = constant 0 (or a loop back-edge).
	DataType valueType(Varnode v, int depth, Set<Varnode> seen) {
		if (v == null || depth > 16) return null;
		if (v.isConstant()) return v.getOffset() == 0 ? NULLPTR : null;
		if (!seen.add(v)) return NULLPTR;
		PcodeOp d = v.getDef();
		if (d == null) {
			HighVariable h = v.getHigh();
			if (v.isInput() && h instanceof HighParam && h.getHighFunction().getFunction().getSignatureSource() != SourceType.DEFAULT) return h.getDataType();
			return null;
		}
		switch (d.getOpcode()) {
			case PcodeOp.COPY: case PcodeOp.CAST: case PcodeOp.INDIRECT:
				return valueType(d.getInput(0), depth + 1, seen);
			case PcodeOp.CALL: {
				Function cf = getFunctionAt(d.getInput(0).getAddress());
				if (cf == null || cf.getSignatureSource() == SourceType.DEFAULT) return null;
				DataType rt = cf.getReturnType();
				return rt == null || rt.getLength() != v.getSize() ? null : rt;
			}
			case PcodeOp.MULTIEQUAL: {
				DataType best = null;
				for (Varnode in : d.getInputs()) {
					DataType t = valueType(in, depth + 1, seen);
					if (t == null) return null;
					if (t == NULLPTR) continue;
					if (best == null) { best = t; continue; }
					String a = ptName(best), b = ptName(t);
					if (a == null || b == null) { if (!best.isEquivalent(t)) return null; continue; }
					if (a.equals(b) || isBase(a, b, 0)) best = t;   // keep the base-most
					else if (!isBase(b, a, 0)) return null;
				}
				return best == null ? NULLPTR : best;
			}
			case PcodeOp.PTRSUB: {
				if (!d.getInput(1).isConstant()) return null;
				DataType bt = valueType(d.getInput(0), depth + 1, seen);
				if (bt == null || bt == NULLPTR) return null;
				Structure st = pointee(bt);
				if (st == null) return null;
				DataTypeComponent c = st.getComponentAt((int) d.getInput(1).getOffset());
				if (c == null || c.getOffset() != d.getInput(1).getOffset() || c.getFieldName() == null) return null;
				return dtm.getPointer(c.getDataType());
			}
			case PcodeOp.LOAD: {
				long off = 0; Varnode a = d.getInput(1);
				for (int i = 0; i < 16; i++) {   // address = base + constant offsets
					PcodeOp e = a.getDef();
					if (e == null) break;
					int oc = e.getOpcode();
					if ((oc == PcodeOp.PTRSUB || oc == PcodeOp.INT_ADD) && e.getInput(1).isConstant()) off += e.getInput(1).getOffset();
					else if (oc != PcodeOp.CAST && oc != PcodeOp.COPY) break;
					a = e.getInput(0);
				}
				DataType bt = valueType(a, depth + 1, seen);
				if (bt == null || bt == NULLPTR) return null;
				DataType t = pointee(bt);
				for (int i = 0; i < 16 && t != null; i++) {   // descend to the member that starts at off with the loaded size
					t = unTd(t);
					if (off == 0 && t.getLength() == v.getSize() && !(t instanceof Structure)) return t;
					if (!(t instanceof Structure st)) return null;
					DataTypeComponent c = st.getComponentContaining((int) off);
					if (c == null || c.getFieldName() == null || c.getDataType() instanceof Undefined || c.getDataType() instanceof DefaultDataType) return null;
					off -= c.getOffset(); t = c.getDataType();
				}
				return null;
			}
		}
		return null;
	}
	Structure pointee(DataType t) {
		t = unTd(t);
		if (!(t instanceof Pointer p)) return null;
		DataType s = unTd(p.getDataType());
		return s instanceof Structure ss ? ss : null;
	}
	String ptName(DataType t) { Structure s = pointee(t); return s == null ? null : s.getName(); }

	// The PDB's static type of the object in register `reg` (full 64-bit) at text offset pc: the class pointer type of
	// every S_LOCAL (of the function or of an inlined callee) live in that register there; the most derived of them
	// (they hold the same pointer, so the object is at least that); null if none, or if any two are unrelated.
	String pdbClassAt(PF pf, String reg, long pc) {
		String best = null;
		for (PV p : pf.vars) {
			if (p.parent != null) continue;
			for (int i = 0; i < p.reg.size(); i++) {
				long[] g = p.reg.get(i);
				if (!p.regName.get(i).equals(reg) || g[0] != 0 || g[1] != 8) continue;
				if (!liveAt(pc, g[2], g[3], p.regGaps.get(i))) continue;
				String t = p.type.replace("const ", "").replace("volatile ", "").trim();
				if (!t.endsWith("*")) return null;
				t = t.substring(0, t.length() - 1).trim();
				if (t.endsWith("*") || !(dtm.getDataType(pdbCat, dtName(t)) instanceof Structure)) return null;
				t = dtName(t);
				if (best == null || isBase(t, best, 0)) best = t;
				else if (!t.equals(best) && !isBase(best, t, 0)) return null;
			}
		}
		return best;
	}

	int nRetyped = 0;
	// primary (offset-0, super_) base chain of a struct, most derived first
	List<String> chain(String s) {
		List<String> out = new ArrayList<>();
		for (int i = 0; i < 40 && s != null; i++) {
			out.add(s);
			DataType d = dtm.getDataType(pdbCat, s);
			String nx = null;
			if (d instanceof Structure st) {
				DataTypeComponent c = st.getComponentAt(0);
				if (c != null && c.getOffset() == 0 && c.getFieldName() != null && c.getFieldName().startsWith("super_")) nx = c.getDataType().getName();
			}
			s = nx;
		}
		return out;
	}
	String commonBase(List<String> xs) {
		if (xs.isEmpty()) return null;
		for (String c : chain(xs.get(0))) {
			boolean all = true;
			for (String x : xs) if (!x.equals(c) && !isBase(x, c, 0)) { all = false; break; }
			if (all) return c;
		}
		return null;
	}
	int retypeConflicts(HighFunction hf) {
		List<Object[]> todo = new ArrayList<>();
		Iterator<HighSymbol> it = hf.getLocalSymbolMap().getSymbols();
		while (it.hasNext()) {
			HighSymbol hs = it.next();
			if (hs.isParameter()) continue;
			String nm = hs.getName();
			if (nm == null || !AUTO.matcher(nm).matches()) continue;
			HighVariable hv = hs.getHighVariable();
			String D = ptName(hs.getDataType());
			if (hv == null || D == null) continue;
			List<String> srcs = new ArrayList<>();
			boolean unknown = false, conflict = false;
			for (Varnode vn : hv.getInstances()) {
				PcodeOp def = vn.getDef();
				if (def != null && def.getOpcode() == PcodeOp.MULTIEQUAL) continue;
				DataType vt = valueType(vn, 0, new HashSet<>());
				if (vt == NULLPTR) continue;
				String S = vt == null ? null : ptName(vt);
				if (S == null) { unknown = true; continue; }
				srcs.add(S);
				if (!related(S, D)) conflict = true;
			}
			if (!conflict) continue;
			String cb = unknown ? null : commonBase(srcs);
			DataType nt = cb != null ? dtm.getPointer(dtm.getDataType(pdbCat, cb)) : dtm.getPointer(VoidDataType.dataType);
			todo.add(new Object[] { hs, nt });
		}
		int n = 0;
		for (Object[] t : todo) {
			try { HighFunctionDBUtil.updateDBVariable((HighSymbol) t[0], null, (DataType) t[1], SourceType.ANALYSIS); n++; }
			catch (Exception e) { if (debugNames) println("retype " + ((HighSymbol) t[0]).getName() + ": " + e); }
		}
		return n;
	}

	// the register the vtable pointer was loaded through (MOV Rx, qword ptr [Ry]) and, walking back inside the basic
	// block over plain register copies (MOV Ry, Rz), the PDB class type of the object it holds there
	// as pdbClassAt, for the locals whose ranged frame record (F = frame pointer + off, B = RSP/RBP + off) puts them at
	// Ghidra stack offset so, live at pc
	String pdbStackClassAt(PF pf, long[] fr, long so, long pc) {
		String best = null;
		for (PV p : pf.vars) {
			if (p.parent != null) continue;
			for (int i = 0; i < p.fr.size(); i++) {
				long[] g = p.fr.get(i);
				String b = p.frBase.get(i);
				long bo = b.equals("RSP") ? fr[0] : b.equals("RBP") ? fr[1] : Long.MIN_VALUE;
				if (bo == Long.MIN_VALUE || bo + g[0] != so || g[1] < 0) continue;
				if (!liveAt(pc, g[1], g[2], p.frGaps.get(i))) continue;
				String t = p.type.replace("const ", "").replace("volatile ", "").trim();
				if (!t.endsWith("*")) return null;
				t = t.substring(0, t.length() - 1).trim();
				if (t.endsWith("*") || !(dtm.getDataType(pdbCat, dtName(t)) instanceof Structure)) return null;
				t = dtName(t);
				if (best == null || isBase(t, best, 0)) best = t;
				else if (!t.equals(best) && !isBase(best, t, 0)) return null;
			}
		}
		return best;
	}

	String lastEvid = null;   // where pdbObjectClass found its PDB local: R,<reg>,<pc>  or  K,<stack off>,<pc>,<rsp>,<rbp>
	String pdbObjectClass(PF pf, Function f, Address loadAt) {
		if (pf == null) return null;
		Instruction ins = getInstructionAt(loadAt);
		if (ins == null || !ins.getMnemonicString().equals("MOV") || ins.getNumOperands() != 2) return null;
		Object[] mo = ins.getOpObjects(1);
		if (mo.length != 1 || !(mo[0] instanceof Register r) || r.getBitLength() != 64 || !ghidra.program.model.lang.OperandType.isDynamic(ins.getOperandType(1))) return null;
		String reg = r.getName();
		ReferenceManager rm = currentProgram.getReferenceManager();
		for (int step = 0; step < 24 && ins != null && f.getBody().contains(ins.getAddress()); step++) {
			String t = pdbClassAt(pf, reg, toff(ins.getAddress()));
			if (t != null) { lastEvid = "R," + reg + "," + toff(ins.getAddress()); return t; }
			boolean blockStart = false;
			for (Reference x : rm.getReferencesTo(ins.getAddress())) if (x.getReferenceType().isFlow()) blockStart = true;
			if (blockStart) return null;
			Instruction pv = ins.getPrevious();
			if (pv == null || pv.getFlowType().isCall() || pv.getFlowType().isJump() || pv.getFlowType().isTerminal()) return null;
			Register cur = currentProgram.getRegister(reg);
			boolean writes = false;
			for (Object o : pv.getResultObjects()) if (o instanceof Register rr && (rr.contains(cur) || cur.contains(rr))) writes = true;
			if (writes) {
				if (!pv.getMnemonicString().equals("MOV")) return null;
				Object[] src = pv.getOpObjects(1), dst = pv.getOpObjects(0);
				// MOV Ry, qword ptr [RSP/RBP + d]: the object came from a stack home; the PDB local whose ranged (not
				// whole-function: slots are reused) frame record holds that slot there
				if (src.length == 2 && src[0] instanceof Register sb && src[1] instanceof Scalar sd && ghidra.program.model.lang.OperandType.isDynamic(pv.getOperandType(1))
						&& (sb.getName().equals("RSP") || sb.getName().equals("RBP")) && dst.length == 1 && dst[0] instanceof Register r3 && r3.getName().equals(reg)) {
					long[] fr = frame(f);
					long base = sb.getName().equals("RSP") ? fr[0] : fr[1];
					if (base == Long.MIN_VALUE) return null;
					long so = base + sd.getSignedValue(), pcs = toff(pv.getAddress());
					String st = pdbStackClassAt(pf, fr, so, pcs);
					if (st != null) { lastEvid = "K," + so + "," + pcs + "," + fr[0] + "," + fr[1]; return st; }
					return null;
				}
				if (src.length != 1 || !(src[0] instanceof Register r2) || r2.getBitLength() != 64 || ghidra.program.model.lang.OperandType.isDynamic(pv.getOperandType(1))) return null;
				if (dst.length != 1 || !(dst[0] instanceof Register r3) || !r3.getName().equals(reg)) return null;
				reg = r2.getName();
				t = pdbClassAt(pf, reg, toff(pv.getAddress()));
				if (t != null) { lastEvid = "R," + reg + "," + toff(pv.getAddress()); return t; }
			}
			ins = pv;
		}
		return null;
	}

	// old decompiler name -> PDB name, for one function.
	// stat: [0] renamed, [1] rejected (no PDB local live over every def/use), [2] rejected (ambiguous: several PDB
	// locals cover it), [3] rejected (type mismatch), [4] names the round-4 rule would have applied that fail this check.
	//
	// Rule (round 5, "a wrong name is worse than no name"): a decompiler variable (HighVariable) gets a PDB local's
	// name only if EVERY def and use of EVERY instance of it lies inside that local's live ranges (S_DEFRANGE_*, minus
	// their gaps) for the instance's exact storage (register lane / frame slot), the PDB local is the only one that does,
	// and the types are compatible. A def is checked at its instruction and the next one (CodeView ranges start after
	// the defining instruction); a use at the reading instruction. Uses by MULTIEQUAL/INDIRECT are not instructions
	// and are skipped (their outputs are instances, checked as defs). Instances in the decompiler's unique space have no
	// machine storage and are skipped; a variable with no located instance is not named. Every applied name is written
	// to the side table (`names`) with its checks so scripts/check_local_names.py can verify it against locals.tsv.
	Map<String, String> localNames(Function f, HighFunction hf, Set<String> taken, int[] stat, StringBuilder names) {
		Map<String, String> out = new HashMap<>();
		PF pf = pdbLocals.get(toff(f.getEntryPoint()));
		if (pf == null || pf.vars.isEmpty()) return out;
		long[] fr = null;
		Map<PV, Integer> used = new HashMap<>();
		Iterator<HighSymbol> it = hf.getLocalSymbolMap().getSymbols();
		while (it.hasNext()) {
			HighSymbol hs = it.next();
			String old = hs.getName();
			if (old == null || !AUTO.matcher(old).matches()) continue;
			HighVariable hv = hs.getHighVariable();
			List<Varnode> inst = new ArrayList<>();
			if (hv != null) inst.addAll(Arrays.asList(hv.getInstances()));
			VariableStorage vs = hs.getStorage();
			if (inst.isEmpty() && vs != null) for (Varnode v : vs.getVarnodes()) inst.add(v);
			// checkpoints per located instance: {storage kind (0 reg, 1 stack), offset, size, pc, pcNext or -1 for a use}
			List<long[]> cps = new ArrayList<>();
			int located = 0, unlocated = 0;
			List<DataType> vts = new ArrayList<>();   // value types of the instances' definitions (valueType)
			for (Varnode vn : inst) {
				Address va = vn.getAddress();
				int kind = vn.isRegister() ? 0 : va.isStackAddress() ? 1 : -1;
				long so = va.getOffset();
				boolean defIsUse = false;
				PcodeOp def = vn.getDef();
				if (kind < 0 && def != null && def.getOpcode() == PcodeOp.COPY && mergeOnly(vn, hv)) {
					// the decompiler's own COPY that only feeds this variable's joins (MULTIEQUAL/INDIRECT whose outputs are
					// located instances of it, each checked in its own storage): an assignment `V = <other value>` on one
					// path; the input is a read of the other value, not of V. Its value type still counts.
					DataType vt = valueType(vn, 0, new HashSet<>());
					if (vt != null && vt != NULLPTR) vts.add(vt);
					continue;
				}
				if (kind < 0) {
					// round 6: an instance in the decompiler's unique space is checked in the machine storage it is proven
					// to occupy (uniqueStorage); if there is none, the variable is not named
					long[] us = uniqueStorage(vn, 0);
					if (us == null) { unlocated++; continue; }
					kind = (int) us[0]; so = us[1]; defIsUse = us[2] == 1;
				}
				located++;
				if (kind == 1 && fr == null) fr = frame(f);
				if (def == null || def.getOpcode() != PcodeOp.MULTIEQUAL) {   // a MULTIEQUAL's inputs are instances themselves
					DataType vt = valueType(vn, 0, new HashSet<>());
					if (vt != null && vt != NULLPTR) vts.add(vt);
				}
				long pc, pcNext;
				if (def != null) {
					Address t = def.getSeqnum().getTarget();
					pc = toff(t);
					Instruction ins = getInstructionAt(t);
					pcNext = defIsUse ? -1 : ins == null ? pc : pc + ins.getLength();
				}
				else { pc = toff(f.getEntryPoint()); pcNext = pc; }
				long uq = vn.isUnique() ? 1 : 0;   // (debug only) checkpoint of a unique-space instance
				cps.add(new long[] { kind, so, vn.getSize(), pc, pcNext, uq });
				Iterator<PcodeOp> ds = vn.getDescendants();
				while (ds != null && ds.hasNext()) {
					PcodeOp u = ds.next();
					int oc = u.getOpcode();
					if (oc == PcodeOp.MULTIEQUAL || oc == PcodeOp.INDIRECT) continue;
					cps.add(new long[] { kind, so, vn.getSize(), toff(u.getSeqnum().getTarget()), -1, uq });
				}
			}
			if (located == 0) continue;
			// the PDB locals (of this function, not of an inlined callee) live over every checkpoint
			List<PV> cand = new ArrayList<>();
			for (PV p : pf.vars) {
				if (!p.inl.isEmpty()) continue;
				boolean all = true;
				for (long[] c : cps) if (!live(p, c, fr)) { all = false; break; }
				if (all) cand.add(p);
			}
			if (debugNames && cand.isEmpty()) {   // which names the unique-space checkpoints removed, and why
				List<PV> c5 = new ArrayList<>();
				for (PV p : pf.vars) {
					if (!p.inl.isEmpty()) continue;
					boolean all = true;
					for (long[] c : cps) if (c[5] == 0 && !live(p, c, fr)) { all = false; break; }
					if (all) c5.add(p);
				}
				if (c5.size() == 1) {
					StringBuilder db = new StringBuilder("DBG lost " + old + " -> " + c5.get(0).name + ":");
					for (long[] c : cps) if (c[5] == 1 && !live(c5.get(0), c, fr))
						db.append(" | ").append(c[0] == 0 ? regPdbForm(c[1], (int) c[2]) : "K" + c[1]).append(" pc=").append(Long.toHexString(c[3] + 0x1000)).append(c[4] < 0 ? " use " : " def ").append(getInstructionAt(at(c[3])));
					println(db.toString());
				}
			}
			// round-4 rule, kept only to count what it would have named wrongly: majority of instances matching at their def
			PV r4 = r4Rule(pf, inst, f, fr, hs);
			PV best = null;
			if (cand.size() == 1) best = cand.get(0);
			else if (cand.size() > 1) {
				List<PV> same = new ArrayList<>();
				for (PV p : cand) if (p.param == hs.isParameter()) same.add(p);
				if (same.size() == 1) best = same.get(0);
			}
			if (best == null) { stat[cand.isEmpty() ? 1 : 2]++; if (r4 != null) stat[4]++; continue; }
			if (r4 != null && r4 != best) stat[4]++;
			// round 6: an instance with no machine storage (the decompiler's unique space) has no PC/storage to check
			// against the live ranges; it can be a different source variable merged in (ProcessHitForDamage: the
			// attacker's vehicle pawn merged into DefenderHorse's R12), so such a variable is never named
			if (unlocated > 0) {
				stat[5]++;
				if (debugNames) {
					StringBuilder db = new StringBuilder("DBG unlocated " + old + " -> " + best.name + ":");
					for (Varnode vn : inst) {
						if (vn.isRegister() || vn.getAddress().isStackAddress()) continue;
						PcodeOp d0 = vn.getDef();
						db.append(" | inst ").append(vn).append(" def=").append(d0 == null ? "-" : d0.toString());
						if (d0 != null) { Instruction ii = getInstructionAt(d0.getSeqnum().getTarget()); db.append(" @").append(ii); }
						Iterator<PcodeOp> ds = vn.getDescendants();
						while (ds != null && ds.hasNext()) { PcodeOp u = ds.next(); db.append(" | use ").append(u).append(" @").append(getInstructionAt(u.getSeqnum().getTarget())); }
					}
					println(db.toString());
				}
				continue;
			}
			String nm;
			if (best.parent != null) {
				// a member of an aggregate local that lives in a register: <Local>_<member path>, e.g. Dir_Y
				Object[] mp = member(best.parent.type, best.poff);
				char ca = mp == null ? 'x' : ghCat((DataType) mp[1]), cb = ghCat(hs.getDataType());
				if (mp == null || (ca != '?' && cb != '?' && ca != cb)) { stat[3]++; continue; }
				if (((DataType) mp[1]).getLength() != hs.getSize()) { stat[6]++; continue; }   // a piece of the member, or more than it
				boolean bad = false;
				for (DataType vt : vts) { char cv = ghCat(vt); if (ca != '?' && cv != '?' && cv != ca) bad = true; }
				if (bad) { stat[7]++; continue; }
				nm = (best.name + "_" + mp[0]).replaceAll("[^A-Za-z0-9_]", "_");
			}
			else {
				if (!typeOk(best, hs.getDataType())) { stat[3]++; continue; }
				// round 6: the decompiler variable must be the whole local, not a piece of it (an 8-byte undefined8 at the
				// slot of a 12-byte FVector) nor more than it
				// (a register range records its own byte size, and the checks above matched it exactly: an int kept
				// sign-extended in a 64-bit register is the local; the type size is the measure for stack slots)
				long pz = pdbSize(best.type);
				boolean onStack = false;
				for (long[] c : cps) if (c[0] == 1) onStack = true;
				if (onStack && pz > 0 && pz != hs.getSize()) { stat[6]++; continue; }
				// round 6: no definition may assign a value whose own type contradicts the PDB type (another category, or
				// a pointer to a class unrelated to the PDB's class)
				boolean bad = false;
				for (DataType vt : vts) if (!typeOk(best, vt)) bad = true;
				if (bad) { stat[7]++; continue; }
				nm = best.name.replaceAll("[^A-Za-z0-9_]", "_");
			}
			if (nm.equals("this") && !hs.isParameter()) { stat[3]++; continue; }   // a copy of `this`: the decompiler already prints this
			if (nm.isEmpty() || Character.isDigit(nm.charAt(0)) || (RESERVED.contains(nm) && !nm.equals("this"))) nm = "v_" + nm;
			int k = used.merge(best, 1, Integer::sum);
			if (k > 1) nm = nm + "_" + k;   // the PDB variable was split by the decompiler into several pieces
			while (taken.contains(nm)) nm = nm + "_";
			taken.add(nm);
			out.put(old, nm);
			stat[0]++;
			if (names != null) {
				names.append("0x").append(Long.toHexString(0x1000 + toff(f.getEntryPoint()))).append('\t').append(nm).append('\t')
					.append(best.name).append('\t').append(best.parent == null ? -1 : best.poff).append('\t')
					.append(fr == null ? "-" : fr[0] + "," + fr[1]).append('\t');
				StringBuilder cs = new StringBuilder();
				for (long[] c : cps) {
					if (cs.length() > 0) cs.append(';');
					String st = c[0] == 0 ? "R," + regPdbForm(c[1], (int) c[2]) : "K," + c[1];
					cs.append(st).append(',').append(c[2]).append(',').append(c[3]).append(',').append(c[4]);
				}
				// value types of the definitions: T,<category F/I/P/S/?>,<size>,<pointee struct or ->
				Set<String> seenT = new TreeSet<>();
				for (DataType vt : vts) {
					String pn = ptName(vt);
					seenT.add("T," + ghCat(vt) + "," + vt.getLength() + "," + (pn == null ? "-" : pn.replace(";", "_").replace(",", "_")));
				}
				for (String t : seenT) cs.append(';').append(t);
				names.append(cs).append('\n');
			}
		}
		return out;
	}

	// a register varnode as locals.tsv writes registers: <64-bit GPR or XMMn>,<byte offset in it> (EAX -> RAX,0;
	// XMM6_Db -> XMM6,4), found by walking Ghidra's parent registers
	String regPdbForm(long off, int size) {
		Address a = currentProgram.getAddressFactory().getRegisterSpace().getAddress(off);
		Register r = currentProgram.getRegister(a, size);
		if (r == null) r = currentProgram.getRegister(a);
		while (r != null && !r.getName().matches("R(AX|BX|CX|DX|SI|DI|BP|SP|8|9|1[0-5])|XMM\\d+")) r = r.getParentRegister();
		if (r == null) return "?,0";
		return r.getName() + "," + (off - r.getAddress().getOffset());
	}

	static boolean inGaps(long pc, long start, long[] gaps) {
		for (int i = 0; i + 1 < gaps.length; i += 2) if (pc >= start + gaps[i] && pc < start + gaps[i] + gaps[i + 1]) return true;
		return false;
	}
	static boolean liveAt(long pc, long start, long len, long[] gaps) { return inR(pc, start, len) && !inGaps(pc, start, gaps); }

	// is PDB local p live in the checkpoint's storage at its pc (a def: at pc or the next instruction)?
	boolean live(PV p, long[] c, long[] fr) {
		long pc = c[3], pcNext = c[4];
		if (c[0] == 0) {
			long o = c[1], e = o + c[2];
			for (int i = 0; i < p.reg.size(); i++) {
				long[] g = p.reg.get(i);
				Register R = currentProgram.getRegister(p.regName.get(i));
				if (R == null) continue;
				long ro = R.getAddress().getOffset() + g[0], re = ro + g[1];
				if (o != ro || e != re) continue;
				long[] gp = p.regGaps.get(i);
				if (liveAt(pc, g[2], g[3], gp) || (pcNext >= 0 && liveAt(pcNext, g[2], g[3], gp))) return true;
			}
			return false;
		}
		long o = c[1];
		for (int i = 0; i < p.fr.size(); i++) {
			long[] g = p.fr.get(i);
			String b = p.frBase.get(i);
			long bo = b.equals("RSP") ? fr[0] : b.equals("RBP") ? fr[1] : Long.MIN_VALUE;
			if (bo == Long.MIN_VALUE || bo + g[0] != o) continue;   // slot start must match
			if (g[1] < 0) return true;   // whole function
			long[] gp = p.frGaps.get(i);
			if (liveAt(pc, g[1], g[2], gp) || (pcNext >= 0 && liveAt(pcNext, g[1], g[2], gp))) return true;
		}
		return false;
	}

	// the round-4 rule (majority of instances matching at their def PC, winner covers >= half, types ok): returns the
	// PDB local it would have named the variable after, or null
	PV r4Rule(PF pf, List<Varnode> inst, Function f, long[] fr, HighSymbol hs) {
		Map<PV, Integer> vote = new LinkedHashMap<>();
		int located = 0;
		for (Varnode vn : inst) {
			int kind = vn.isRegister() ? 0 : vn.getAddress().isStackAddress() ? 1 : -1;
			if (kind < 0) continue;
			located++;
			if (kind == 1 && fr == null) fr = frame(f);
			PcodeOp def = vn.getDef();
			long pc, pcNext;
			if (def != null) {
				Address t = def.getSeqnum().getTarget();
				pc = toff(t);
				Instruction ins = getInstructionAt(t);
				pcNext = ins == null ? pc : pc + ins.getLength();
			}
			else { pc = toff(f.getEntryPoint()); pcNext = pc; }
			long[] c = { kind, vn.getAddress().getOffset(), vn.getSize(), pc, pcNext };
			for (PV p : pf.vars) if (p.inl.isEmpty() && p.parent == null && liveNoGaps(p, c, fr, def == null)) vote.merge(p, 1, Integer::sum);
		}
		PV best = null; int bs = -1, bv = 0;
		for (Map.Entry<PV, Integer> e : vote.entrySet()) {
			int sc = e.getValue() * 2 + (hs.isParameter() == e.getKey().param ? 1 : 0);
			if (sc > bs) { bs = sc; best = e.getKey(); bv = e.getValue(); }
		}
		if (best == null || bv * 2 < located || !typeOk(best, hs.getDataType())) return null;
		return best;
	}
	boolean liveNoGaps(PV p, long[] c, long[] fr, boolean input) {   // round-4 matching: gaps ignored, input stack slots always match
		if (c[0] == 0) {
			for (int i = 0; i < p.reg.size(); i++) {
				long[] g = p.reg.get(i);
				Register R = currentProgram.getRegister(p.regName.get(i));
				if (R == null) continue;
				long ro = R.getAddress().getOffset() + g[0];
				if (c[1] == ro && c[1] + c[2] == ro + g[1] && (inR(c[3], g[2], g[3]) || inR(c[4], g[2], g[3]))) return true;
			}
			return false;
		}
		for (int i = 0; i < p.fr.size(); i++) {
			long[] g = p.fr.get(i);
			String b = p.frBase.get(i);
			long bo = b.equals("RSP") ? fr[0] : b.equals("RBP") ? fr[1] : Long.MIN_VALUE;
			if (bo == Long.MIN_VALUE || bo + g[0] != c[1]) continue;
			if (g[1] < 0 || input || inR(c[3], g[1], g[2]) || inR(c[4], g[1], g[2])) return true;
		}
		return false;
	}

	// ------------------------------------------------------------------ virtual calls

	static class VCall { boolean argIsObj; Varnode root; String name, cast, cls, evid; int slot; Address site; FunctionDefinitionDataType sig; Set<PcodeOp> chain = new HashSet<>(); }
	int[] vstat = new int[6];   // virtual calls: [0] class from the PDB local live in the object's register, [1] from the value's own type, [2] slot past the class's own vftable named by the closed-world table, [3] unproven class (left raw), [4] slot unnamed, [5] printed with a cast

	// ---- virtual-slot signatures (types/vtbl_types.json, scripts/native_vtbl_types.py) -> call-site overrides
	Map<String, Map<Integer, JsonArray>> vtblSigs = null;   // Ghidra struct name -> slot -> [slot, name, ret, params]
	Map<String, Map<Integer, JsonArray>> extSlots = new HashMap<>();   // Ghidra struct name -> slot past its vftable -> [slot, name, intro, ret, params]

	String dtName(String n) {   // same as ApplyTypes.dtName
		if (DataUtilities.isValidDataTypeName(n)) return n;
		String s = n.replaceAll("\\s+", "_");
		return DataUtilities.isValidDataTypeName(s) ? s : s.replaceAll("[^A-Za-z0-9_<>,:*&\\[\\]]", "_");
	}

	void loadVtbl(Path p) throws IOException {
		vtblSigs = new HashMap<>();
		try (Reader r = Files.newBufferedReader(p, StandardCharsets.UTF_8)) {
			JsonObject o = JsonParser.parseReader(r).getAsJsonObject().getAsJsonObject("vtbl");
			for (Map.Entry<String, JsonElement> e : o.entrySet()) {
				Map<Integer, JsonArray> m = new HashMap<>();
				for (JsonElement x : e.getValue().getAsJsonArray()) m.put(x.getAsJsonArray().get(0).getAsInt(), x.getAsJsonArray());
				vtblSigs.put(dtName(e.getKey()), m);
			}
			JsonObject x = JsonParser.parseReader(Files.newBufferedReader(p, StandardCharsets.UTF_8)).getAsJsonObject().getAsJsonObject("ext");
			if (x != null) for (Map.Entry<String, JsonElement> e : x.entrySet()) {
				Map<Integer, JsonArray> m = new HashMap<>();
				for (JsonElement y : e.getValue().getAsJsonArray()) m.put(y.getAsJsonArray().get(0).getAsInt(), y.getAsJsonArray());
				extSlots.put(dtName(e.getKey()), m);
			}
		}
	}

	DataType prim(String n, int z) {   // same table as ApplyTypes.prim
		switch (n) {
			case "void": return VoidDataType.dataType;
			case "bool": return BooleanDataType.dataType;
			case "char": return CharDataType.dataType;
			case "int8": return SignedByteDataType.dataType;
			case "uint8": return ByteDataType.dataType;
			case "int16": return ShortDataType.dataType;
			case "uint16": return UnsignedShortDataType.dataType;
			case "int32": case "long": case "HRESULT": return IntegerDataType.dataType;
			case "uint32": case "unsigned long": return UnsignedIntegerDataType.dataType;
			case "int64": return LongLongDataType.dataType;
			case "uint64": return UnsignedLongLongDataType.dataType;
			case "float": return FloatDataType.dataType;
			case "double": return DoubleDataType.dataType;
			case "wchar_t": case "char16_t": return WideChar16DataType.dataType;
			case "char32_t": return WideChar32DataType.dataType;
		}
		return z > 0 ? Undefined.getUndefinedDataType(z) : VoidDataType.dataType;
	}

	// type ref (native_types.tref JSON) -> a DataType already in the program (ApplyTypes made the PDB layouts);
	// unknown by-value UDTs become undefined bytes of their size, unknown pointees void
	DataType ref(JsonObject t) {
		if (t.has("p")) return prim(t.get("p").getAsString(), t.has("z") ? t.get("z").getAsInt() : 0);
		if (t.has("s") || t.has("e")) {
			DataType d = dtm.getDataType(pdbCat, dtName(t.has("s") ? t.get("s").getAsString() : t.get("e").getAsString()));
			int z = t.has("z") ? t.get("z").getAsInt() : 0;
			if (d != null && (d.getLength() == z || z == 0)) return d;
			return z > 0 ? Undefined.getUndefinedDataType(z) : null;
		}
		if (t.has("ptr")) {
			JsonObject r = t.getAsJsonObject("ptr");
			DataType in = r.has("fn") ? VoidDataType.dataType : ref(r);
			if (in == null) in = VoidDataType.dataType;
			return dtm.getPointer(in);
		}
		return null;
	}

	FunctionDefinitionDataType slotSig(String cls, String name, JsonArray row, DataType thisT) {
		DataType rt = ref(row.get(2).getAsJsonObject());
		if (rt == null) return null;
		JsonArray ps = row.get(3).getAsJsonArray();
		ParameterDefinition[] pd = new ParameterDefinition[ps.size()];
		for (int i = 0; i < ps.size(); i++) {
			DataType t = i == 0 && thisT != null ? thisT : ref(ps.get(i).getAsJsonObject());
			if (t == null || t.getLength() <= 0) return null;
			pd[i] = new ParameterDefinitionImpl(null, t, null);   // unnamed: the decompiler would name arguments after them
		}
		FunctionDefinitionDataType fd = new FunctionDefinitionDataType(CategoryPath.ROOT, (cls + "::" + name).replaceAll("[^A-Za-z0-9_:]", "_"), dtm);
		fd.setReturnType(rt);
		fd.setArguments(pd);
		try { fd.setCallingConvention("__fastcall"); } catch (Exception e) {}
		return fd;
	}


	Varnode strip(Varnode v, Set<PcodeOp> chain) {   // through CAST/COPY and PTRSUB(x, 0) (base sub-object) to the variable
		for (int i = 0; i < 16 && v != null; i++) {
			PcodeOp d = v.getDef();
			if (d == null) return v;
			int oc = d.getOpcode();
			if (oc == PcodeOp.CAST || oc == PcodeOp.COPY) { chain.add(d); v = d.getInput(0); continue; }
			if (oc == PcodeOp.PTRSUB && d.getInput(1).isConstant() && d.getInput(1).getOffset() == 0) { chain.add(d); v = d.getInput(0); continue; }
			return v;
		}
		return v;
	}

	Structure structOf(Varnode v) {
		HighVariable h = v.getHigh();
		DataType t = h == null ? null : h.getDataType();
		if (t instanceof TypeDef td) t = td.getBaseDataType();
		if (!(t instanceof Pointer p)) return null;
		DataType s = p.getDataType();
		if (s instanceof TypeDef td) s = td.getBaseDataType();
		return s instanceof Structure ss ? ss : null;
	}

	List<VCall> vcalls(HighFunction hf) {
		List<VCall> out = new ArrayList<>();
		Iterator<PcodeOpAST> it = hf.getPcodeOps();
		while (it.hasNext()) {
			PcodeOp op = it.next();
			if (op.getOpcode() != PcodeOp.CALLIND) continue;
			VCall c = new VCall();
			Varnode t = op.getInput(0);
			PcodeOp d = t.getDef();
			while (d != null && (d.getOpcode() == PcodeOp.CAST || d.getOpcode() == PcodeOp.COPY)) { c.chain.add(d); t = d.getInput(0); d = t.getDef(); }
			if (d == null || d.getOpcode() != PcodeOp.LOAD) continue;
			c.chain.add(d);
			Varnode a = d.getInput(1);
			long off = 0;
			for (int i = 0; i < 8; i++) {   // slot address = vtbl + off
				PcodeOp e = a.getDef();
				if (e == null) break;
				int oc = e.getOpcode();
				if (oc == PcodeOp.PTRSUB && e.getInput(1).isConstant()) off += e.getInput(1).getOffset();
				else if (oc == PcodeOp.PTRADD && e.getInput(1).isConstant() && e.getInput(2).isConstant()) off += e.getInput(1).getOffset() * e.getInput(2).getOffset();
				else if (oc == PcodeOp.INT_ADD && e.getInput(1).isConstant()) off += e.getInput(1).getOffset();
				else if (oc != PcodeOp.CAST && oc != PcodeOp.COPY) break;
				c.chain.add(e); a = e.getInput(0);
			}
			PcodeOp vl = a.getDef();   // vtbl pointer = *(obj + 0)
			if (vl == null || vl.getOpcode() != PcodeOp.LOAD) continue;
			c.chain.add(vl);
			Varnode obj = strip(vl.getInput(1), c.chain);
			// `this` argument = the object whose vtable was read (MSVC x64: RCX). A call the decompiler printed with no
			// arguments (its prototype unknown) still gets the slot's PDB prototype as an override, so the re-decompile
			// shows `this` and the call can then be rewritten; it is not rewritten before that.
			boolean hasArg = op.getNumInputs() >= 2;
			Varnode arg = hasArg ? strip(op.getInput(1), new HashSet<>()) : null;
			if (obj == null) continue;
			boolean same = arg != null && (arg == obj || (obj.getHigh() != null && obj.getHigh() == arg.getHigh()));
			if (hasArg && !same) continue;
			c.argIsObj = same;
			// Round 6: the object's class is taken from proofs only, never from the decompiler's (merged, guessed) variable
			// type: (a) the PDB local live in the register the vtable pointer was loaded through (the source's own
			// static type there), else (b) the value's own type (valueType: call returns, typed members, parameters).
			// If both exist they must be related. The slot is then named from that class's vftable, or, past its end,
			// from the closed-world table (extSlots, native_vtbl_types.py). A slot past the printed type's own vftable,
			// or a printed type unrelated to the proven class, is printed through a cast: ((Class *)obj)->Name(...).
			if (off % 8 != 0) continue;
			lastEvid = null;
			String pdbT = pdbObjectClass(pdbLocals.get(toff(hf.getFunction().getEntryPoint())), hf.getFunction(), vl.getSeqnum().getTarget());
			DataType vty = valueType(obj, 0, new HashSet<>());
			String valT = vty == null || vty == NULLPTR ? null : ptName(vty);
			String T = pdbT != null ? pdbT : valT;
			if (pdbT != null && valT != null && !related(pdbT, valT)) T = null;
			if (debugNames) println("DBG vcall @" + op.getSeqnum().getTarget() + " off=0x" + Long.toHexString(off) + " pdb=" + pdbT + " (" + lastEvid + ") value=" + valT + " vty=" + vty + " obj=" + obj + " def=" + obj.getDef());
			if (T == null) { vstat[3]++; continue; }
			int slot = (int) (off / 8);
			DataType vt = dtm.getDataType(pdbCat, T + "_vtbl");
			Map<Integer, JsonArray> sigs = vtblSigs == null ? null : vtblSigs.get(T);
			JsonArray row = null; String castTo = null;
			if (vt instanceof Structure vs && off < vs.getLength()) {
				DataTypeComponent cmp = vs.getComponentAt((int) off);
				if (cmp == null || cmp.getFieldName() == null || cmp.getOffset() != off) { vstat[4]++; continue; }
				row = sigs == null ? null : sigs.get(slot);
				c.name = row != null ? row.get(1).getAsString() : cmp.getFieldName().replaceAll("_\\d+$", "").replace("dtor_", "~");
				castTo = T;
			}
			else {
				JsonArray ex = extSlots.getOrDefault(T, Map.of()).get(slot);
				if (ex == null) { vstat[4]++; continue; }
				c.name = ex.get(1).getAsString(); castTo = dtName(ex.get(2).getAsString());
				if (!ex.get(3).isJsonNull() && !ex.get(4).isJsonNull()) { row = new JsonArray(); row.add(slot); row.add(c.name); row.add(ex.get(3)); row.add(ex.get(4)); }
				vstat[2]++;
			}
			vstat[pdbT != null ? 0 : 1]++;
			// no cast when the printed type has this slot as the same method: it is castTo or derived from it, or a base
			// of it whose own vftable already holds the slot
			Structure gst = structOf(obj);
			if (gst != null) {
				String g = gst.getName();
				DataType gv = dtm.getDataType(pdbCat, g + "_vtbl");
				if (g.equals(castTo) || isBase(g, castTo, 0) || (isBase(castTo, g, 0) && gv instanceof Structure gs && off < gs.getLength())) castTo = null;
			}
			c.cast = castTo;
			c.cls = T; c.slot = slot; c.evid = pdbT != null ? lastEvid : "V";
			c.root = obj;
			c.site = op.getSeqnum().getTarget();
			if (row != null) {
				// the override's `this` keeps the object's printed type when it is related to the proven class (a
				// slot owner's `this` would make the decompiler retype the whole merged variable to a derived class)
				DataType thisT = gst != null && related(gst.getName(), T) ? dtm.getPointer(gst) : null;
				c.sig = slotSig(castTo != null ? castTo : T, c.name, row, thisT);
			}
			out.add(c);
		}
		return out;
	}

	// `(TARGET)(obj, rest)` -> `obj->Name(rest)` as token overrides; false when the printed shape is not that
	boolean rewrite(List<ClangToken> toks, Map<ClangToken, String> ov, VCall c, Map<String, String> ren) {
		int lo = Integer.MAX_VALUE, hi = -1;
		for (int i = 0; i < toks.size(); i++) {
			PcodeOp o = toks.get(i).getPcodeOp();
			if (o != null && c.chain.contains(o)) { lo = Math.min(lo, i); hi = Math.max(hi, i); }
		}
		if (hi < 0) { if (debugNames) println("DBG rw fail hi " + c.site); return false; }
		// the `)` that ends the target expression: the first unmatched `)` after the chain that is followed by the
		// argument list `(` (round 6: the chain can end inside inner parentheses, `((longlong)X.vfptr + 0x158))(this)`)
		int close = -1, depth = 0, argOpen = -1;
		for (int i = hi + 1; i < toks.size(); i++) {
			String s = toks.get(i).getText();
			if (s.equals("(")) depth++;
			else if (s.equals(")")) {
				if (depth > 0) { depth--; continue; }
				int j = i + 1;
				while (j < toks.size() && toks.get(j).getText().isBlank()) j++;
				if (j < toks.size() && toks.get(j).getText().equals("(")) { close = i; argOpen = j; break; }
				if (j < toks.size() && !toks.get(j).getText().equals(")")) break;   // the target expression ended otherwise
			}
			else if (s.equals(";") || s.equals(",")) break;
		}
		if (close < 0) { if (debugNames) println("DBG rw fail close " + c.site); return false; }
		if (argOpen >= toks.size() || !toks.get(argOpen).getText().equals("(")) { if (debugNames) println("DBG rw fail argopen " + c.site + " tok=" + (argOpen < toks.size() ? toks.get(argOpen).getText() : "-") + " hitok=" + toks.get(hi).getText() + " closeprev=" + toks.get(close - 1).getText()); return false; }
		int open = -1; depth = 0;   // its matching `(`
		for (int i = close - 1; i >= 0; i--) {
			String s = toks.get(i).getText();
			if (s.equals(")")) depth++;
			else if (s.equals("(")) { if (depth == 0) { open = i; break; } depth--; }
		}
		if (open < 0 || open > lo) { if (debugNames) println("DBG rw fail open " + c.site + " open=" + open + " lo=" + lo + " hi=" + hi + " close=" + close + " lotok=" + toks.get(lo).getText() + " hitok=" + toks.get(hi).getText()); return false; }
		int a0e = -1; depth = 0; boolean more = false;   // end of the first argument
		for (int i = argOpen + 1; i < toks.size(); i++) {
			String s = toks.get(i).getText();
			if (s.equals("(")) depth++;
			else if (s.equals(")")) { if (depth == 0) { a0e = i; break; } depth--; }
			else if (s.equals(",") && depth == 0) { a0e = i; more = true; break; }
		}
		if (a0e < 0) return false;
		if (!c.argIsObj) return false;
		String root = c.root.getHigh() == null ? null : c.root.getHigh().getName();
		if (root == null || root.equals("UNNAMED")) {
			// an unnamed object (e.g. `this->LateTickComponent`): its printed text is the call's first argument
			StringBuilder ab = new StringBuilder();
			for (int i = argOpen + 1; i < a0e; i++) {
				ClangToken t = toks.get(i);
				String x = t.getText();
				ab.append(t instanceof ClangVariableToken && ren.containsKey(x) ? ren.get(x) : x);
			}
			root = ab.toString().trim();
			if (root.isEmpty()) return false;
			if (!root.matches("[A-Za-z_][A-Za-z0-9_]*(->[A-Za-z_][A-Za-z0-9_]*)*")) root = "(" + root + ")";
		}
		else root = ren.getOrDefault(root, root);
		for (int i = open; i <= close; i++) ov.put(toks.get(i), "");
		ov.put(toks.get(open), c.cast == null ? root + "->" + c.name : "((" + c.cast + " *)" + root + ")->" + c.name);
		if (c.cast != null) vstat[5]++;
		for (int i = argOpen + 1; i < a0e; i++) ov.put(toks.get(i), "");
		if (more) {
			ov.put(toks.get(a0e), "");
			if (a0e + 1 < toks.size() && toks.get(a0e + 1).getText().equals(" ")) ov.put(toks.get(a0e + 1), "");
		}
		return true;
	}

	// Same text as DecompileResults.getDecompiledFunction(): PrettyPrinter's lines (empty lines padded with a spacer)
	// and its getText(): IllegalCharCppTransformer.simplify on function/variable/type/field/label tokens that are not
	// constants. Plus our overrides: renamed variables (ren) and rewritten token spans (ov).
	static final IllegalCharCppTransformer XF = new IllegalCharCppTransformer();
	String print(Function f, ClangTokenGroup g, Map<String, String> ren, Map<ClangToken, String> ov) {
		List<String> out = new ArrayList<>();
		boolean joinNext = false;   // previous line ended in a rewritten `obj->Name`: pull the wrapped `(args)` up
		for (ClangLine line : new PrettyPrinter(f, g, XF).getLines()) {
			StringBuilder ln = new StringBuilder();
			boolean changed = false, head = false, rew = false;
			for (ClangToken t : line.getAllTokens()) {
				String s = ov.get(t);
				if (s != null) {
					changed = true; ln.append(s);
					if (!s.isEmpty()) { head = s.contains("->"); rew |= head; }
					continue;
				}
				s = t.getText();
				if (!s.isBlank()) head = false;
				if (t instanceof ClangVariableToken && ren.containsKey(s)) { ln.append(ren.get(s)); continue; }
				boolean clean = t instanceof ClangFuncNameToken || t instanceof ClangVariableToken || t instanceof ClangTypeToken ||
					t instanceof ClangFieldToken || t instanceof ClangLabelToken;
				if (clean && t.getSyntaxType() == ClangToken.CONST_COLOR) clean = false;
				ln.append(clean ? XF.simplify(s) : s);
			}
			if (changed && ln.toString().isBlank()) continue;   // a wrapped line emptied by a rewrite
			String txt = ln.toString();
			if (joinNext && !out.isEmpty() && txt.stripLeading().matches("[(;),].*")) {
				String prev = out.remove(out.size() - 1);
				out.add(prev + txt.stripLeading());
			}
			else out.add(line.getIndentString() + txt);
			joinNext = rew;   // a line holding a rewritten call: pull up the wrapped rest of that call
		}
		StringBuilder sb = new StringBuilder();
		for (String l : out) sb.append(l).append(StringUtilities.LINE_SEPARATOR);
		return sb.toString();
	}

	// ------------------------------------------------------------------ journal (crash-safe, resumable output)
	// The decompiled text is appended to a journal one address group at a time and flushed, instead of being held in
	// memory until the end (a 9,836-group run holds ~160 MB of text and loses everything when killed). A rerun with
	// the same journal skips every group already completed. scripts/ghidra_assemble.py turns the journal into the
	// per-class .cpp files. Record:  "@@G\t<key>\t<status>\t<n>\n" then n x ("@@E\t<file>\t<utf8 bytes>\n" + bytes),
	// then "@@D\t<key>\n". A trailing incomplete group (killed mid-write) is truncated away on resume.
	Path journalP = null; Map<Long, Integer> skipTyped = new HashMap<>(); int ftmo = 0; Path r1Dir = null;

	Set<String> journalDone(Path p) throws IOException {
		Set<String> done = new HashSet<>();
		if (!Files.exists(p)) return done;
		long good = 0;
		try (RandomAccessFile f = new RandomAccessFile(p.toFile(), "rw")) {
			String cur = null;
			while (true) {
				String l = f.readLine();
				if (l == null) break;
				String[] c = l.split("\t");
				if (c[0].equals("@@G") && c.length >= 2) cur = c[1];
				else if (c[0].equals("@@E") && c.length >= 3) {
					long n = Long.parseLong(c[2]);
					if (f.getFilePointer() + n > f.length()) break;
					f.seek(f.getFilePointer() + n);
				}
				else if (c[0].equals("@@D") && cur != null && c.length >= 2 && c[1].equals(cur)) { done.add(cur); good = f.getFilePointer(); cur = null; }
				else break;
			}
			if (f.length() != good) { println("journal: truncating incomplete tail " + (f.length() - good) + " bytes"); f.setLength(good); }
		}
		return done;
	}

	// Round-1 (untyped, no ApplyTypes) body of the function at rva from the r1 output, the last-resort fallback.
	String r1Body(String name, long rva) {
		if (r1Dir == null) return null;
		Path p = r1Dir.resolve(fileOf(name) + ".cpp");
		if (!Files.exists(p)) return null;
		try {
			List<String> ls = Files.readAllLines(p, StandardCharsets.UTF_8);
			String tag = "  rva=0x" + Long.toHexString(rva) + " size=";
			StringBuilder sb = null;
			for (String l : ls) {
				boolean head = l.startsWith("// ") && l.contains("  rva=0x") && l.contains(" size=");
				if (sb != null) { if (head) break; sb.append(l).append(StringUtilities.LINE_SEPARATOR); }
				else if (head && l.contains(tag)) sb = new StringBuilder();
			}
			if (sb == null) return null;
			String s = sb.toString().replaceAll("(\\r?\\n)+$", "") + StringUtilities.LINE_SEPARATOR;
			return s.contains("DECOMPILE FAILED") ? null : s;
		} catch (IOException e) { return null; }
	}

	// Decompile with types stripped: this function's parameters/return become undefined8 and unlocked, and so do the
	// prototypes of every function it calls; everything is restored afterwards.
	DecompileResults stripped(DecompInterface d, Function f, int t) throws Exception {
		Map<Function, Object[]> was = new LinkedHashMap<>();
		List<Function> fs = new ArrayList<>(); fs.add(f);
		for (Function cf : f.getCalledFunctions(monitor)) if (cf != f) fs.add(cf);
		for (Function x : fs) {
			List<Parameter> ps = new ArrayList<>();
			for (Parameter p : x.getParameters()) ps.add(new ParameterImpl(p.getName(), p.getDataType(), currentProgram, p.getSource()));
			was.put(x, new Object[] { x.getCallingConventionName(), x.getReturnType(), ps, x.getSignatureSource(), x.hasVarArgs() });
			List<Parameter> un = new ArrayList<>();
			for (int i = 0; i < ps.size(); i++) un.add(new ParameterImpl("param_" + (i + 1), Undefined8DataType.dataType, currentProgram, SourceType.DEFAULT));
			try { x.updateFunction(x.getCallingConventionName(), new ReturnParameterImpl(Undefined8DataType.dataType, currentProgram), un, Function.FunctionUpdateType.DYNAMIC_STORAGE_ALL_PARAMS, true, SourceType.DEFAULT); } catch (Exception e) {}
			x.setSignatureSource(SourceType.DEFAULT);
		}
		try { return d.decompileFunction(f, t, monitor); }
		finally {
			for (Map.Entry<Function, Object[]> e : was.entrySet()) {
				Object[] o = e.getValue(); Function x = e.getKey();
				@SuppressWarnings("unchecked") List<Parameter> ps = (List<Parameter>) o[2];
				try { x.updateFunction((String) o[0], new ReturnParameterImpl((DataType) o[1], currentProgram), ps, Function.FunctionUpdateType.DYNAMIC_STORAGE_ALL_PARAMS, true, (SourceType) o[3]); } catch (Exception ex) { println("restore " + x.getName() + ": " + ex); }
				x.setVarArgs((Boolean) o[4]);
				x.setSignatureSource((SourceType) o[3]);
			}
		}
	}

	// canonical name of an ICF group: the one whose class is the typed `this` (ApplyTypes typed the first PDB name),
	// else the first PDB name
	String[] canonOf(Function f, List<String[]> g) {
		if (g.size() > 1 && f != null && f.getParameterCount() > 0 && f.getParameter(0).getName().equals("this")) {
			DataType tt = f.getParameter(0).getDataType();
			String cn = tt instanceof Pointer pp ? pp.getDataType().getName() : "";
			for (String[] r : g) if (owner(r[0]).equals(cn) || clean(owner(r[0])).equals(cn)) return r;
		}
		return g.get(0);
	}

	DecompInterface newDecomp() {
		DecompInterface d = new DecompInterface();
		// default payload is too small for the largest game functions ("Response buffer size exceeded")
		DecompileOptions opt = new DecompileOptions();
		opt.setMaxPayloadMBytes(512);
		d.setOptions(opt);
		d.toggleCCode(true);
		d.toggleSyntaxTree(true);
		d.openProgram(currentProgram);
		return d;
	}

	// ------------------------------------------------------------------ main
	@Override
	public void run() throws Exception {
		String[] a = getScriptArgs();
		Path labels = Paths.get(a[0]), funcs = Paths.get(a[1]), out = Paths.get(a[2]);
		int tmo = a.length > 3 ? Integer.parseInt(a[3]) : 60;
		String mode = a.length > 4 ? a[4] : "all";
		Path dataP = null, localsP = null, vtblP = null;
		for (int i = 5; i < a.length; i++) {
			if (a[i].startsWith("data=")) dataP = Paths.get(a[i].substring(5));
			if (a[i].startsWith("locals=")) localsP = Paths.get(a[i].substring(7));
			if (a[i].startsWith("vtbl=")) vtblP = Paths.get(a[i].substring(5));
			if (a[i].startsWith("journal=")) journalP = Paths.get(a[i].substring(8));
			if (a[i].startsWith("skiptyped=")) for (String s : Files.readAllLines(Paths.get(a[i].substring(10)))) { s = s.replaceAll("#.*", "").trim(); if (!s.isEmpty()) { String[] q = s.split("\\s+"); skipTyped.put(Long.decode(q[0]), q.length > 1 ? Integer.parseInt(q[1]) : 2); } }
			if (a[i].startsWith("ftmo=")) ftmo = Integer.parseInt(a[i].substring(5));
			if (a[i].startsWith("r1=")) r1Dir = Paths.get(a[i].substring(3));
		}
		if (ftmo <= 0) ftmo = Math.min(tmo, 300);
		text = currentProgram.getMemory().getBlock(".text").getStart();
		dtm = currentProgram.getDataTypeManager();
		SymbolTable st = currentProgram.getSymbolTable();

		// 1. labels for every PDB function entry in .text (engine + game), then global data
		int nl = 0;
		if (!mode.equals("decomp"))
		for (String l : Files.readAllLines(labels, StandardCharsets.UTF_8)) {
			String[] p = l.split("\t", 2);
			try { st.createLabel(at(Long.parseLong(p[0])), clean(p[1]), SourceType.IMPORTED); nl++; } catch (Exception e) {}
		}
		println("labels: " + nl);
		if (!mode.equals("decomp") && dataP != null) println("data labels: " + dataLabels(dataP));

		// 2. disassemble + create every game function at its PDB address/size
		List<String[]> rows = new ArrayList<>();
		List<String> fl = Files.readAllLines(funcs, StandardCharsets.UTF_8);
		for (int i = 1; i < fl.size(); i++) rows.add(fl.get(i).split("\t"));
		for (String[] r : rows) {
			Address s = at(Long.parseLong(r[2]));
			if (getInstructionAt(s) == null) new DisassembleCommand(s, null, true).applyTo(currentProgram, monitor);
			if (getFunctionAt(s) == null) new CreateFunctionCmd(s).applyTo(currentProgram, monitor);
			Function f = getFunctionAt(s);
			if (f != null && !mode.equals("decomp")) try { f.setName(clean(r[0]), SourceType.IMPORTED); } catch (Exception e) {}
		}
		println("functions created: " + rows.size());
		if (!mode.equals("decomp")) {
			println("exception funclet prototypes: " + funcletPrototypes(rows));
			int[] c = calleeFunctions(rows);
			println("callee functions created: " + c[0] + ", already functions: " + c[1] + ", skipped (no PDB entry / not code): " + c[2]);
		}
		if (mode.equals("prepare")) return;
		if (localsP != null) { loadLocals(localsP); println("PDB locals for " + pdbLocals.size() + " functions"); }
		if (vtblP != null) { loadVtbl(vtblP); println("virtual-slot signatures for " + vtblSigs.size() + " classes"); }

		// 3. decompile; one .cpp per owning class (text before the last ::), like a source file. Output goes to the
		// journal group by group (see journalDone); without journal= it is kept in memory and written at the end.
		DecompInterface d = newDecomp();
		Map<String, StringBuilder> files = new TreeMap<>();
		Set<String> done = journalP == null ? new HashSet<>() : journalDone(journalP);
		if (journalP != null) println("journal " + journalP + ": " + done.size() + " groups already done, resuming");
		OutputStream jo = journalP == null ? null : new BufferedOutputStream(new FileOutputStream(journalP.toFile(), true), 1 << 16);
		int ok = 0, bad = 0, icf = 0, vok = 0, vmiss = 0, mism = 0, untyped = 0, ovr = 0, ovbad = 0, fromR1 = 0, skipped = 0, sinceReset = 0;
		int[] ls = new int[8];
		// Linker identical-COMDAT folding (/OPT:ICF): several functions can share one address and one body.
		Map<String, List<String[]>> shared = new LinkedHashMap<>();
		for (String[] r : rows) shared.computeIfAbsent(r[2], k -> new ArrayList<>()).add(r);
		// Name every group's Function by its canonical name BEFORE decompiling anything: game functions are also
		// callees of other game functions (template instantiations, each module's operator new/delete), so renaming
		// them inside the loop made a callee's printed name depend on decompile order (and on where a resumed run
		// started). Checked: re-decompiling a sample on the saved project reproduces the run byte for byte.
		for (Map.Entry<String, List<String[]>> grp : shared.entrySet()) {
			Function f = getFunctionAt(at(Long.parseLong(grp.getKey())));
			if (f != null) try { f.setName(clean(canonOf(f, grp.getValue())[0]), SourceType.IMPORTED); } catch (Exception e) {}
		}
		int gi = 0;
		for (Map.Entry<String, List<String[]>> grp : shared.entrySet()) {
			if (monitor.isCancelled()) break;
			gi++;
			if (done.contains(grp.getKey())) { skipped++; continue; }
			// a fresh decompiler process every 400 groups keeps decompile.exe's memory bounded
			if (++sinceReset >= 400) { d.dispose(); d = newDecomp(); sinceReset = 0; }
			List<String[]> g = grp.getValue();
			Function f = getFunctionAt(at(Long.parseLong(grp.getKey())));
			long rva = 0x1000 + Long.parseLong(grp.getKey());
			// canonical name: the one whose class is the typed `this` (ApplyTypes typed the first PDB name), else first
			String[] canon = canonOf(f, g);
			String body = null, note = "", status = "ok";
			StringBuilder namesSb = new StringBuilder();   // applied PDB local names + their live-range checks (side table)
			StringBuilder vcallsSb = new StringBuilder();  // rewritten virtual calls + their evidence (side table)
			// Determinism: the output must never depend on whether a timeout fired in this run. A function listed in
			// skiptyped.txt (rva + fallback level) is decompiled ONLY at that level, once. Any other timeout (typed
			// attempt, or the vtable-override re-decompile) still yields the most typed body we can get, but its group is
			// journaled with status "timeout"; ghidra_assemble.py then fails the build and names the rva + the level to
			// add to skiptyped.txt, so a timeout can only ever make a run fail, not make it print something different.
			Integer lvlFixed = skipTyped.get(rva);
			DecompileResults res = f == null || lvlFixed != null ? null : d.decompileFunction(f, tmo, monitor);
			if (f != null && (res == null || !res.decompileCompleted())) {
				// Fallback levels: 1 this function's PDB prototype unlocked; 2 also its callees' prototypes unlocked;
				//  3 its own and its callees' parameter/return types stripped to undefined8 (round-1 conditions);
				//  4 the round-1 body (r1= dir, untyped run without ApplyTypes). Everything is restored afterwards.
				String err = lvlFixed != null ? "listed in skiptyped: typed decompile never terminates (r2: 900 s and 1800 s)" : res == null ? "no result" : res.getErrorMessage().trim().replaceAll("\\s+", " ");
				if (err.isEmpty()) err = "timeout after " + tmo + " s / empty error";
				String lvl = null;
				int from = lvlFixed != null ? lvlFixed : 1, to = lvlFixed != null ? lvlFixed : 4, used = 0;
				for (int L = from; L <= to && (res == null || !res.decompileCompleted()); L++) {
					used = L;
					if (L > from) { d.dispose(); d = newDecomp(); sinceReset = 0; }   // a timed-out decompiler can be left in a bad state
					int t = lvlFixed != null ? tmo : ftmo;
					if (L == 1 || L == 2) {
						Map<Function, SourceType> was = new LinkedHashMap<>();
						was.put(f, f.getSignatureSource());
						f.setSignatureSource(SourceType.DEFAULT);
						if (L == 2) for (Function cf : f.getCalledFunctions(monitor)) if (!was.containsKey(cf)) { was.put(cf, cf.getSignatureSource()); cf.setSignatureSource(SourceType.DEFAULT); }
						try { res = d.decompileFunction(f, t, monitor); }
						finally { for (Map.Entry<Function, SourceType> e : was.entrySet()) e.getKey().setSignatureSource(e.getValue()); }
						lvl = L == 1 ? "this function's PDB prototype unlocked (untyped this/params)"
							: "the PDB prototypes of this function and its " + (was.size() - 1) + " callees unlocked (untyped this/params/returns)";
					}
					else if (L == 3) {
						res = stripped(d, f, t);
						lvl = "the parameter/return types of this function and its callees stripped to undefined8 (untyped)";
					}
					else {
						String b1 = r1Body(canon[0], rva);
						res = null;
						if (b1 != null) {
							body = b1; fromR1++;
							note = "// NOTE: the typed decompile failed (" + err + ") and so did every untyped retry; this is the round-1 body\n"
								+ "// (same Ghidra, no PDB types applied: extract/native/decomp_r1), so names/types below are raw.\n";
						}
						break;
					}
				}
				if (body == null) note = "// NOTE: the typed decompile failed (" + err + "); this body was decompiled with " + lvl + "\n";
				untyped++;
				status = lvlFixed != null ? "untyped" : "timeout L" + used;
				if (lvlFixed == null) println("TIMEOUT " + canon[0] + " rva=0x" + Long.toHexString(rva) + ": typed decompile failed (" + err + "), body from fallback level " + used + "; add `0x" + Long.toHexString(rva) + " " + used + "` to skiptyped.txt");
			}
			// (e') virtual calls resolved to Class::Slot get that slot's PDB prototype as a call-site override, then the
			// function is decompiled once more so return values and arguments are typed (`UWorld *` from GetWorld)
			if (vtblSigs != null && res != null && res.decompileCompleted() && res.getHighFunction() != null && status.equals("ok")) {
				int n = 0;
				for (VCall v : vcalls(res.getHighFunction())) if (v.sig != null) try { HighFunctionDBUtil.writeOverride(f, v.site, v.sig); n++; } catch (Exception ex) { if (ovbad++ < 10) println("override " + canon[0] + ": " + ex); }
				if (n > 0) {
					DecompileResults r2 = d.decompileFunction(f, tmo, monitor);
					if (r2 != null && r2.decompileCompleted()) { res = r2; ovr += n; }
					else { status = "timeout override"; println("TIMEOUT " + canon[0] + " rva=0x" + Long.toHexString(rva) + ": re-decompile with vtable-slot overrides failed; body printed without them"); }
				}
			}
			// (f) round 6: a variable printed as a pointer to class D that is assigned a value whose own type (valueType) is a
			// pointer to a class unrelated to D cannot be D for that value (ProcessHitForDamage: `this_07` printed
			// AMordhauCharacter* holds a UMotionSystemComponent*). The decompiler merged several values into it and took
			// one value's type. Such a variable is committed to the nearest common base class of all its values (or
			// void * if any value's type is unknown), and the function is decompiled once more.
			if (res != null && res.decompileCompleted() && res.getHighFunction() != null && status.equals("ok")) {
				int n = retypeConflicts(res.getHighFunction());
				if (n > 0) {
					DecompileResults r3 = d.decompileFunction(f, tmo, monitor);
					if (r3 != null && r3.decompileCompleted()) { res = r3; nRetyped += n; }
					else { status = "timeout retype"; println("TIMEOUT " + canon[0] + " rva=0x" + Long.toHexString(rva) + ": re-decompile with retyped variables failed"); }
				}
			}
			if (res != null && res.decompileCompleted()) {
				String c = res.getDecompiledFunction().getC();
				body = c;
				ClangTokenGroup mk = res.getCCodeMarkup();
				HighFunction hf = res.getHighFunction();
				if (mk != null && hf != null) {
					try {
						String pr = print(f, mk, Map.of(), Map.of());
						if (!pr.equals(c)) {
							if (mism++ < 3) {
								int i = 0;
								while (i < pr.length() && i < c.length() && pr.charAt(i) == c.charAt(i)) i++;
								println("printer mismatch " + canon[0] + " at " + i + ": getC=[" + c.substring(Math.max(0, i - 40), Math.min(c.length(), i + 40)).replace("\n", "\\n") + "] ours=[" + pr.substring(Math.max(0, i - 40), Math.min(pr.length(), i + 40)).replace("\n", "\\n") + "]");
							}
						}
						else {
							List<ClangNode> all = new ArrayList<>();
							mk.flatten(all);
							List<ClangToken> toks = new ArrayList<>();
							Set<String> taken = new HashSet<>();
							for (ClangNode n : all) if (n instanceof ClangToken t) { toks.add(t); taken.add(t.getText()); }
							StringBuilder nsb = new StringBuilder();
							Map<String, String> ren = localNames(f, hf, taken, ls, nsb);
							Map<ClangToken, String> ov = new HashMap<>();
							StringBuilder vsb = new StringBuilder();
							for (VCall v : vcalls(hf)) {
								if (rewrite(toks, ov, v, ren)) {
									vok++;
									// side table for scripts/check_local_names.py: rva, call site, slot, printed name, class it was named
									// from, cast printed (or -), evidence (R,<reg>,<pc>: the PDB local live there; V: the value's own type)
									vsb.append(String.join("\t", "0x" + Long.toHexString(rva), "0x" + Long.toHexString(0x1000 + toff(v.site)),
										String.valueOf(v.slot), v.name, v.cls, v.cast == null ? "-" : v.cast, v.evid)).append('\n');
								}
								else vmiss++;
							}
							vcallsSb.append(vsb);
							body = print(f, mk, ren, ov);
							namesSb.append(nsb);   // only names that made it into the printed body
						}
					} catch (Exception ex) { body = c; println("print " + canon[0] + ": " + ex); }
				}
			}
			if (body != null) ok++; else { bad++; status = "bad"; }
			List<String[]> ents = new ArrayList<>();   // {file, text}
			for (String[] r : g) {
				StringBuilder sb = new StringBuilder();
				sb.append("// ").append(r[0]).append(kind(r[0])).append("  rva=0x").append(Long.toHexString(rva)).append(" size=").append(r[3]).append(" obj=").append(r[4]).append("\n");
				if (g.size() > 1) {
					sb.append("// ICF-folded: the linker merged ").append(g.size()).append(" functions with identical machine code into this address:\n");
					for (String[] o : g) if (o != r) sb.append("//   ").append(o[0]).append(o == canon ? "   <- body printed under this name" : "").append("\n");
				}
				if (body == null) sb.append("// DECOMPILE FAILED: ").append(res == null ? "no function" : res.getErrorMessage()).append("\n\n");
				else if (r == canon) sb.append(note).append(body).append("\n");
				else { sb.append("// body: see ").append(canon[0]).append(" in ").append(fileOf(canon[0])).append(".cpp (identical code; this name's own source body is not recoverable)\n\n"); icf++; }
				ents.add(new String[] { fileOf(r[0]), sb.toString() });
			}
			if (namesSb.length() > 0) ents.add(new String[] { "@locals", namesSb.toString() });   // -> _local_names.tsv (ghidra_assemble.py)
			if (vcallsSb.length() > 0) ents.add(new String[] { "@vcalls", vcallsSb.toString() });   // -> _vcalls.tsv
			if (jo != null) {
				jo.write(("@@G\t" + grp.getKey() + "\t" + status + "\t" + ents.size() + "\n").getBytes(StandardCharsets.UTF_8));
				for (String[] e : ents) {
					byte[] b = e[1].getBytes(StandardCharsets.UTF_8);
					jo.write(("@@E\t" + e[0] + "\t" + b.length + "\n").getBytes(StandardCharsets.UTF_8));
					jo.write(b);
				}
				jo.write(("@@D\t" + grp.getKey() + "\n").getBytes(StandardCharsets.UTF_8));
				jo.flush();
			}
			else for (String[] e : ents) files.computeIfAbsent(e[0], k -> new StringBuilder()).append(e[1]);
			if ((ok + bad) % 250 == 0) println("decompiled " + (ok + bad) + " this session, group " + gi + "/" + shared.size() + " (resumed past " + skipped + ")");
		}
		d.dispose();
		if (jo != null) jo.close();
		else {
			Files.createDirectories(out);
			for (Map.Entry<String, StringBuilder> e : files.entrySet())
				Files.writeString(out.resolve(e.getKey().equals("@locals") ? "_local_names.tsv" : e.getKey().equals("@vcalls") ? "_vcalls.tsv" : e.getKey() + ".cpp"), e.getValue().toString(), StandardCharsets.UTF_8);
		}
		println("decompile ok=" + ok + " failed=" + bad + " this session, resumed past " + skipped + " groups (distinct addresses of " + rows.size() + " functions: " + shared.size() + ") icf-references=" + icf + " untyped-fallback=" + untyped + " round-1-body-fallback=" + fromR1);
		println("PDB local names applied: " + ls[0] + " (rejected: no PDB local live over every def/use " + ls[1] + ", ambiguous " + ls[2] + ", type mismatch " + ls[3] + ", instance without storage " + ls[5] + ", size not the whole local " + ls[6] + ", assigned value of another type " + ls[7] + "); names the round-4 rule would have applied that fail the live-range check: " + ls[4] + "; virtual calls rewritten=" + vok + " not=" + vmiss + " typed by slot prototype=" + ovr + "; printer mismatches (fell back to getC)=" + mism);
		println("variables retyped to the common base of their values (class conflict): " + nRetyped);
		println("virtual-call classes (per decompile pass): from the PDB local in the object's register " + vstat[0] + ", from the value's own type " + vstat[1] + ", slot past the class's vftable named closed-world " + vstat[2] + ", class unproven (left raw) " + vstat[3] + ", slot unnamed " + vstat[4] + "; rewritten calls printed through a cast " + vstat[5]);
	}
}
