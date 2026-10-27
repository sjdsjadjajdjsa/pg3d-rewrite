// Applies names/types recovered by Cpp2IL (via scripts/cs_index.py) to GameAssembly.dll.
//   - creates a function at every method RVA that has none, names it Namespace::Class::Method
//   - extra labels for methods that share one body (identical code folded by the compiler)
//   - plate comment with the C# signature + metadata token
//   - builds Ghidra structs for game classes from Cpp2IL field offsets (category /Il2Cpp)
// Usage (headless): -postScript ApplyCpp2IL.java <path to private/index>
//@category Il2Cpp

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.data.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import ghidra.app.util.NamespaceUtils;
import ghidra.program.database.function.OverlappingFunctionException;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

public class ApplyCpp2IL extends GhidraScript {

    static final int MAX_LABELS_PER_ADDR = 4;
    static final Set<String> STRUCT_ASMS = Set.of("Assembly-CSharp", "Assembly-CSharp-firstpass", "UnityEngine.CoreModule");

    record Method(long rva, long len, String asm, String type, String name, String token, String sig) {}
    record TypeRec(String asm, String full, String kind, String base) {}
    record FieldRec(String type, String name, String ftype, int offset, boolean isStatic) {}

    Map<String, TypeRec> types = new HashMap<>();
    Map<String, List<String>> bySimpleName = new HashMap<>();
    Map<String, StructureDataType> structs = new HashMap<>();
    Map<String, List<FieldRec>> fieldsByType = new HashMap<>();
    Set<String> filled = new HashSet<>();
    DataTypeManager dtm;

    @Override
    protected void run() throws Exception {
        String[] args = getScriptArgs();
        Path dir = Paths.get(args.length > 0 ? args[0] : askDirectory("Cpp2IL index dir", "Use").getPath());
        dtm = currentProgram.getDataTypeManager();
        loadTypes(dir.resolve("types.tsv"));
        loadFields(dir.resolve("fields.tsv"));
        buildStructs();
        applyMethods(dir.resolve("methods.tsv"));
    }

    // ---------- helpers ----------

    static List<String> rows(Path p) throws IOException {
        List<String> l = Files.readAllLines(p, StandardCharsets.UTF_8);
        return l.subList(1, l.size());
    }

    /** split on '.' but not inside <...> generic brackets */
    static List<String> splitPath(String s) {
        List<String> out = new ArrayList<>();
        int depth = 0;
        StringBuilder cur = new StringBuilder();
        for (char c : s.toCharArray()) {
            if (c == '<') depth++;
            if (c == '>') depth--;
            if (c == '.' && depth == 0) { out.add(cur.toString()); cur.setLength(0); }
            else cur.append(c);
        }
        out.add(cur.toString());
        return out;
    }

    static String clean(String s) {
        return SymbolUtilities.replaceInvalidChars(s.replace(", ", ",").replace(' ', '_'), true);
    }

    String simple(String full) {
        List<String> p = splitPath(full);
        return p.get(p.size() - 1);
    }

    // ---------- types / structs ----------

    void loadTypes(Path p) throws IOException {
        // asm full raw kind mods base token obf methods fields code_bytes file
        for (String line : rows(p)) {
            String[] c = line.split("\t", -1);
            TypeRec t = new TypeRec(c[0], c[1], c[3], c[5]);
            types.put(t.full(), t);
            bySimpleName.computeIfAbsent(simple(t.full()), k -> new ArrayList<>()).add(t.full());
            if (!c[2].equals(simple(t.full()))) bySimpleName.computeIfAbsent(c[2], k -> new ArrayList<>()).add(t.full());
        }
    }

    void loadFields(Path p) throws IOException {
        // type name raw ftype offset static token
        for (String line : rows(p)) {
            String[] c = line.split("\t", -1);
            long off = Long.parseLong(c[4].substring(2), 16);
            if (off > 0x100000) continue; // thread-static / special offsets
            fieldsByType.computeIfAbsent(c[0], k -> new ArrayList<>())
                .add(new FieldRec(c[0], c[1], c[3], (int) off, Boolean.parseBoolean(c[5])));
        }
    }

    CategoryPath category(TypeRec t) {
        List<String> parts = splitPath(t.full());
        CategoryPath cp = new CategoryPath("/Il2Cpp");
        for (int i = 0; i < parts.size() - 1; i++) cp = new CategoryPath(cp, clean(parts.get(i)));
        return cp;
    }

    void buildStructs() {
        monitor.setMessage("Il2Cpp: creating structs");
        for (TypeRec t : types.values()) {
            if (!STRUCT_ASMS.contains(t.asm()) || !t.kind().equals("class")) continue;
            structs.put(t.full(), new StructureDataType(category(t), clean(simple(t.full())) + "_o", 0, dtm));
        }
        int n = 0;
        for (String full : new ArrayList<>(structs.keySet())) {
            if (monitor.isCancelled()) return;
            fill(full, 0);
            n++;
        }
        int id = currentProgram.startTransaction("Il2Cpp structs");
        try {
            for (String full : structs.keySet()) dtm.addDataType(structs.get(full), DataTypeConflictHandler.REPLACE_HANDLER);
        } finally { currentProgram.endTransaction(id, true); }
        println("Il2Cpp: structs created: " + n);
    }

    String resolveBase(TypeRec t) {
        if (t.base().isEmpty()) return null;
        String first = t.base().split(",")[0].trim();
        List<String> cands = bySimpleName.get(first);
        if (cands == null) return null;
        for (String c : cands) if (!c.equals(t.full()) && structs.containsKey(c)) return c;
        return null;
    }

    DataType fieldType(String ft) {
        switch (ft) {
            case "bool": return BooleanDataType.dataType;
            case "byte": case "sbyte": return ByteDataType.dataType;
            case "short": case "ushort": case "char": return ShortDataType.dataType;
            case "int": return IntegerDataType.dataType;
            case "uint": return UnsignedIntegerDataType.dataType;
            case "long": return LongLongDataType.dataType;
            case "ulong": return UnsignedLongLongDataType.dataType;
            case "float": return FloatDataType.dataType;
            case "double": return DoubleDataType.dataType;
            case "IntPtr": case "UIntPtr": return PointerDataType.dataType;
        }
        List<String> c = bySimpleName.get(ft);
        if (c != null && c.size() == 1) {
            TypeRec t = types.get(c.get(0));
            if (t.kind().equals("enum")) return IntegerDataType.dataType;
            if (t.kind().equals("struct")) return null; // value type, size unknown
            StructureDataType s = structs.get(t.full());
            return s != null ? new PointerDataType(s, dtm) : PointerDataType.dataType;
        }
        // string, arrays, generics, interfaces, delegates, unknown classes -> plain pointer
        return PointerDataType.dataType;
    }

    void fill(String full, int depth) {
        if (filled.contains(full) || depth > 40) return;
        filled.add(full);
        StructureDataType s = structs.get(full);
        TypeRec t = types.get(full);
        List<FieldRec> fs = new ArrayList<>();
        for (FieldRec f : fieldsByType.getOrDefault(full, List.of())) if (!f.isStatic()) fs.add(f);
        fs.sort(Comparator.comparingInt(FieldRec::offset));
        String base = resolveBase(t);
        int minOff = fs.isEmpty() ? Integer.MAX_VALUE : fs.get(0).offset();
        int end = 0x10;
        StructureDataType baseS = null;
        if (base != null) {
            fill(base, depth + 1);
            baseS = structs.get(base);
            if (baseS.getLength() <= minOff) end = Math.max(end, baseS.getLength());
            else baseS = null;
        }
        for (FieldRec f : fs) {
            DataType dt = fieldType(f.ftype());
            end = Math.max(end, f.offset() + (dt == null ? 1 : dt.getLength()));
        }
        end = (end + 7) & ~7;
        s.growStructure(end);
        if (baseS != null && baseS.getLength() > 0) {
            s.replaceAtOffset(0, baseS, baseS.getLength(), "base", "inherited from " + base);
        } else {
            s.replaceAtOffset(0, PointerDataType.dataType, 8, "klass", "Il2CppClass*");
            s.replaceAtOffset(8, PointerDataType.dataType, 8, "monitor", null);
        }
        for (int i = 0; i < fs.size(); i++) {
            FieldRec f = fs.get(i);
            DataType dt = fieldType(f.ftype());
            int next = i + 1 < fs.size() ? fs.get(i + 1).offset() : end;
            try {
                if (dt == null || f.offset() + dt.getLength() > next) {
                    s.replaceAtOffset(f.offset(), Undefined1DataType.dataType, 1, clean(f.name()), f.ftype());
                } else {
                    s.replaceAtOffset(f.offset(), dt, dt.getLength(), clean(f.name()), f.ftype());
                }
            } catch (Exception e) {
                // overlapping/odd layout: skip field
            }
        }
    }

    // ---------- methods ----------

    Namespace ns(String typeFull) throws Exception {
        List<String> parts = splitPath(typeFull);
        StringBuilder sb = new StringBuilder();
        for (String p : parts) { if (sb.length() > 0) sb.append("::"); sb.append(clean(p)); }
        return NamespaceUtils.createNamespaceHierarchy(sb.toString(), null, currentProgram, SourceType.IMPORTED);
    }

    void applyMethods(Path p) throws Exception {
        // rva length asm type name raw token sig
        TreeMap<Long, List<Method>> byRva = new TreeMap<>();
        for (String line : rows(p)) {
            String[] c = line.split("\t", -1);
            Method m = new Method(Long.parseLong(c[0].substring(2), 16), Long.parseLong(c[1].substring(2), 16),
                c[2], c[3], c[4], c[6], c[7]);
            byRva.computeIfAbsent(m.rva(), k -> new ArrayList<>()).add(m);
        }
        Address base = currentProgram.getImageBase();
        Listing listing = currentProgram.getListing();
        SymbolTable st = currentProgram.getSymbolTable();
        monitor.initialize(byRva.size());
        monitor.setMessage("Il2Cpp: naming functions");
        int created = 0, named = 0, failed = 0, done = 0;
        int id = currentProgram.startTransaction("Il2Cpp names");
        try {
            for (Map.Entry<Long, List<Method>> e : byRva.entrySet()) {
                if (monitor.isCancelled()) break;
                if (++done % 5000 == 0) {
                    monitor.setProgress(done);
                    currentProgram.endTransaction(id, true);
                    id = currentProgram.startTransaction("Il2Cpp names");
                    println("Il2Cpp: " + done + "/" + byRva.size());
                }
                List<Method> ms = e.getValue();
                // prefer a game-assembly method as the primary name
                ms.sort(Comparator.comparingInt((Method m) -> m.asm().startsWith("Assembly-CSharp") ? 0 : 1));
                Address a = base.add(e.getKey());
                Method first = ms.get(0);
                try {
                    Function f = listing.getFunctionAt(a);
                    String name = clean(first.name());
                    Namespace n = ns(first.type());
                    if (f == null) {
                        disassemble(a);
                        AddressSet body = new AddressSet(a, a.add(Math.max(1, first.len()) - 1));
                        try {
                            f = listing.createFunction(name, n, a, body, SourceType.IMPORTED);
                            created++;
                        } catch (OverlappingFunctionException ofe) {
                            f = null; // another function's body already covers this address
                        }
                    }
                    if (f != null) {
                        f.setParentNamespace(n);
                        f.setName(name, SourceType.IMPORTED);
                    } else {
                        st.createLabel(a, name, n, SourceType.IMPORTED).setPrimary();
                    }
                    for (int i = 1; i < Math.min(ms.size(), MAX_LABELS_PER_ADDR); i++) {
                        Method m = ms.get(i);
                        st.createLabel(a, clean(m.name()), ns(m.type()), SourceType.IMPORTED);
                    }
                    StringBuilder pc = new StringBuilder();
                    pc.append(first.type()).append("\n").append(first.sig()).append("\ntoken 0x").append(first.token())
                      .append("  [").append(first.asm()).append("]");
                    if (ms.size() > 1) pc.append("\nshared body: ").append(ms.size()).append(" methods use this code");
                    listing.setComment(a, CommentType.PLATE, pc.toString());
                    named++;
                } catch (Exception ex) {
                    failed++;
                    if (failed < 20) printerr("Il2Cpp: " + a + " " + first.type() + "::" + first.name() + ": " + ex);
                }
            }
        } finally { currentProgram.endTransaction(id, true); }
        println("Il2Cpp: addresses=" + byRva.size() + " named=" + named + " newFunctions=" + created + " failed=" + failed);
    }
}
