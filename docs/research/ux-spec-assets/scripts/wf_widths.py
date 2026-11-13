import sys, unicodedata
p = sys.argv[1]
lines = open(p, encoding='utf-8').read().split('\n')
inblock = False
start = 0
def w(s):
    return sum(2 if unicodedata.east_asian_width(c) in ('W','F') else 0 if unicodedata.combining(c) else 1 for c in s)
block = []
for i, l in enumerate(lines, 1):
    if l.strip().startswith('```'):
        if inblock:
            widths = {}
            for (n, t) in block:
                if t.strip().startswith(('│','┌','└','├')) or t.startswith(('│','┌','└','├')):
                    widths.setdefault(w(t.rstrip()), []).append(n)
            if len(widths) > 1:
                print(f"block starting line {start}: widths -> " + ", ".join(f"{k}: lines {v[:6]}{'...' if len(v)>6 else ''}" for k, v in sorted(widths.items())))
            block = []
            inblock = False
        else:
            inblock = True
            start = i
        continue
    if inblock:
        block.append((i, l))
