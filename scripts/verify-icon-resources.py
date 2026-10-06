"""Read PE icon resources without executing a program (standard-library only)."""
import struct
import sys
from pathlib import Path

source = (Path(__file__).resolve().parent.parent / "resources/icons/app.ico").read_bytes()
count = struct.unpack_from("<H", source, 4)[0]
expected = set()
for index in range(count):
    size, offset = struct.unpack_from("<II", source, 6 + index * 16 + 8)
    expected.add(source[offset:offset + size])

for filename in sys.argv[1:]:
    data = Path(filename).read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    assert data[pe:pe + 4] == b"PE\0\0"
    optional = pe + 24
    directory = optional + (112 if struct.unpack_from("<H", data, optional)[0] == 0x20B else 96)
    resource_rva = struct.unpack_from("<I", data, directory + 16)[0]
    section_count = struct.unpack_from("<H", data, pe + 6)[0]
    sections = optional + struct.unpack_from("<H", data, pe + 20)[0]

    def address(rva):
        for index in range(section_count):
            virtual_size, start, size, offset = struct.unpack_from("<IIII", data, sections + index * 40 + 8)
            if start <= rva < start + max(size, virtual_size):
                return offset + rva - start
        raise ValueError("Resource address is outside PE sections")

    root = address(resource_rva)
    icons = set()
    groups = 0

    def walk(relative=0, kind=None):
        global groups
        location = root + relative
        named, numbered = struct.unpack_from("<HH", data, location + 12)
        for index in range(named + numbered):
            name, target = struct.unpack_from("<II", data, location + 16 + index * 8)
            entry_kind = name if kind is None else kind
            if target & 0x80000000:
                walk(target & 0x7FFFFFFF, entry_kind)
            else:
                rva, size = struct.unpack_from("<II", data, root + target)
                if entry_kind == 3:
                    start = address(rva)
                    icons.add(data[start:start + size])
                elif entry_kind == 14:
                    groups += 1

    walk()
    assert groups and expected.issubset(icons), f"Approved ICO frames missing from {filename}"
    print(f"{Path(filename).name}: all {count} approved icon frames embedded")
