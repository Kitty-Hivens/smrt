"""
Extract @Mod annotation fields (modid, version, acceptableRemoteVersions,
acceptableSavedVersions, clientSideOnly, serverSideOnly) from each Forge mod
jar by reading the RuntimeVisibleAnnotations attribute on the @Mod-annotated
class. Uses raw .class bytecode parsing -- no javap dependency.
"""
import io, struct, sys, zipfile
from pathlib import Path

MOD_ANNOT_DESC = b"Lnet/minecraftforge/fml/common/Mod;"

class CPReader:
    def __init__(self, blob):
        self.b = blob
        self.i = 0
    def u1(self): v = self.b[self.i]; self.i += 1; return v
    def u2(self): v = struct.unpack_from(">H", self.b, self.i)[0]; self.i += 2; return v
    def u4(self): v = struct.unpack_from(">I", self.b, self.i)[0]; self.i += 4; return v
    def skip(self, n): self.i += n
    def take(self, n): v = self.b[self.i:self.i+n]; self.i += n; return v

def parse_class(blob):
    r = CPReader(blob)
    if r.u4() != 0xCAFEBABE:
        return None
    r.u2(); r.u2()  # minor, major
    cp_count = r.u2()
    cp = [None]
    i = 1
    while i < cp_count:
        tag = r.u1()
        if tag == 1:  # UTF8
            ln = r.u2()
            cp.append(("UTF8", r.take(ln)))
        elif tag == 3 or tag == 4:  # Integer / Float
            r.skip(4); cp.append((tag, None))
        elif tag == 5 or tag == 6:  # Long / Double (takes 2 slots)
            r.skip(8); cp.append((tag, None)); cp.append(None); i += 1
        elif tag == 7:  # Class
            cp.append(("Class", r.u2()))
        elif tag == 8:  # String
            cp.append(("String", r.u2()))
        elif tag in (9, 10, 11):  # Fieldref/Methodref/InterfaceMethodref
            r.skip(4); cp.append((tag, None))
        elif tag == 12:  # NameAndType
            r.skip(4); cp.append((tag, None))
        elif tag == 15:  # MethodHandle
            r.skip(3); cp.append((tag, None))
        elif tag in (16, 19, 20):  # MethodType/Module/Package
            r.skip(2); cp.append((tag, None))
        elif tag == 17 or tag == 18:  # Dynamic/InvokeDynamic
            r.skip(4); cp.append((tag, None))
        else:
            return None
        i += 1

    def utf8(idx):
        e = cp[idx]
        if e and e[0] == "UTF8":
            return e[1]
        return b""

    r.u2()  # access flags
    r.u2(); r.u2()  # this_class, super_class
    ifaces = r.u2()
    r.skip(ifaces * 2)

    def skip_attrs(reader):
        n = reader.u2()
        for _ in range(n):
            reader.u2()  # name
            ln = reader.u4()
            reader.skip(ln)

    fields = r.u2()
    for _ in range(fields):
        r.u2(); r.u2(); r.u2()
        skip_attrs(r)
    methods = r.u2()
    for _ in range(methods):
        r.u2(); r.u2(); r.u2()
        skip_attrs(r)

    # Class-level attributes
    n = r.u2()
    for _ in range(n):
        name_idx = r.u2()
        ln = r.u4()
        body = r.take(ln)
        name = utf8(name_idx)
        if name == b"RuntimeVisibleAnnotations":
            # parse annotations
            br = CPReader(body)
            ac = br.u2()
            for _ in range(ac):
                type_idx = br.u2()
                desc = utf8(type_idx)
                nev = br.u2()
                vals = {}
                for _ in range(nev):
                    el_name = utf8(br.u2()).decode("utf-8", "ignore")
                    tag = br.u1()
                    val = read_element_value(br, tag, utf8)
                    vals[el_name] = val
                if desc == MOD_ANNOT_DESC:
                    return vals
    return None

def read_element_value(br, tag, utf8):
    if tag in (ord("B"), ord("C"), ord("D"), ord("F"), ord("I"), ord("J"), ord("S"), ord("Z")):
        br.u2()  # const_value_index
        return None
    if tag == ord("s"):
        return utf8(br.u2()).decode("utf-8", "ignore")
    if tag == ord("e"):
        br.u2(); br.u2()
        return None
    if tag == ord("c"):
        br.u2()
        return None
    if tag == ord("@"):
        br.u2()  # type
        nev = br.u2()
        for _ in range(nev):
            br.u2(); t = br.u1(); read_element_value(br, t, utf8)
        return None
    if tag == ord("["):
        n = br.u2()
        arr = []
        for _ in range(n):
            t = br.u1()
            arr.append(read_element_value(br, t, utf8))
        return arr
    return None

def scan_jar(jar_path):
    try:
        with zipfile.ZipFile(jar_path) as z:
            for n in z.namelist():
                if not n.endswith(".class"):
                    continue
                try:
                    blob = z.read(n)
                except Exception:
                    continue
                if MOD_ANNOT_DESC not in blob:
                    continue
                vals = parse_class(blob)
                if vals:
                    return vals, n
    except Exception as e:
        return {"err": str(e)}, ""
    return None, ""

mods_dir = Path("/home/haru/.local/share/nexira/clients/Industrial/mods")
rows = []
for jar in sorted(mods_dir.glob("*.jar")):
    vals, where = scan_jar(jar)
    if not vals:
        rows.append((jar.name, "", "", "", "", "", "no @Mod found"))
        continue
    modid = vals.get("modid", "") or ""
    ver = vals.get("version", "") or ""
    are = vals.get("acceptableRemoteVersions", "") or ""
    asv = vals.get("acceptableSavedVersions", "") or ""
    cso = "Y" if vals.get("clientSideOnly") else ""
    sso = "Y" if vals.get("serverSideOnly") else ""
    rows.append((jar.name, modid, ver, are or "(unset)", asv or "(unset)", cso + sso, where))

print(f"{'File':36} {'modid':24} {'version':22} {'acceptRemote':22} {'side':4}")
print("-" * 120)
for r in rows:
    side = ("C" if "Y" in r[5] and "Y" not in r[5][1:] else "") + ("S" if r[5].endswith("Y") and len(r[5]) > 1 else "")
    print(f"{r[0]:36} {r[1]:24} {r[2]:22} {r[3]:22} {r[5]:4}")

# Summary
print("\n=== acceptableRemoteVersions distribution ===")
buckets = {}
for r in rows:
    k = r[3]
    buckets[k] = buckets.get(k, 0) + 1
for k, v in sorted(buckets.items(), key=lambda x: -x[1]):
    print(f"  {v:3} mods : {k}")
