import sys
W = 71  # last column index
def L0():
    a = "┌─ Developer 96.1 GB "
    s = a + "─" * (41 - len(a)) + "┬"
    b = "─ Containers 31.0 GB "
    s += b + "─" * (W - len(s) - len(b)) if False else b
    s += "─" * (W - len(s)) + "┐"
    return s
def blk_top(label, width):
    s = "┌ " + label + " "
    return s + "─" * (width - len(s) - 1) + "┐"
def blk_mid(text, width):
    inner = width - 2
    return "│" + (text + " " * inner)[:inner] + "│"
def blk_bot(width):
    return "└" + "─" * (width - 2) + "┘"
lines = []
lines.append(L0())
# row 1: tops
lines.append("│" + blk_top("Xcode 60.2 GB", 24) + blk_top("CoreSim", 16) + "│" + blk_top("com.apple.x", 16) + blk_top("com.y", 13) + " " * 0 + "│")
lines.append("│" + blk_mid("", 24) + blk_mid(" 30.0 GB", 16) + "│" + blk_mid("", 16) + blk_mid("", 13) + "│")
lines.append("│" + blk_mid("", 24) + blk_mid("", 16) + "│" + blk_bot(16) + blk_bot(13) + "│")
c = "├─ Caches 18.0 GB "
lines.append("│" + blk_mid("", 24) + blk_mid("", 16) + c + "─" * (W - 41 - len(c)) + "┤")
lines.append("│" + blk_mid("", 24) + blk_mid("", 16) + "│" + blk_top("Safari", 15) + blk_top("Spotify", 14) + "│")
lines.append("│" + blk_bot(24) + blk_bot(16) + "│" + blk_bot(15) + blk_bot(14) + "│")
# bottom row: Other groups
a = "├─ Application Support 15.2 GB "
s = a + "─" * (31 - len(a)) + "┬"
s += "─ Logs " + "─" * (41 - len(s) - len("─ Logs ")) + "┴"
s += "─" * (51 - len(s)) + "┬"
b = "─ 12 more "
s += b + "─" * (W - len(s) - len(b)) + "┤"
lines.append(s)
lines.append("│" + " " * 30 + "│" + " " * 19 + "│" + " " * 19 + "│")
lines.append("└" + "─" * 30 + "┴" + "─" * 19 + "┴" + "─" * 19 + "┘")
notes = {0: "← level-1 header strip (18 px), full slot color + label",
         1: "← level-2 blocks: slot at 72%, label if it fits",
         4: "← each top-3 group keeps its own hue",
         7: "← all other top-level folders: neutral gray",
        }
out = []
for i, l in enumerate(lines):
    assert len(l) == W + 1, (i, len(l), l)
    out.append(l + ("   " + notes[i] if i in notes else ""))
out.append(" ■ Developer 96.1 GB  ■ Containers 31.0 GB  ■ Caches 18.0 GB  ■ Other (14) 37.1 GB")
out.append("   ↑ legend row (28 px): swatch + name + size; click selects the folder")
print("\n".join(out))
