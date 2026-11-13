import sys, json
BARS = set('┌┐└┘├┤┬┴┼│╭╮╰╯')
FILL = ' ─━'
def fix_line(line, targets):
    # split into segments
    segs = []; bars = []; cur = ''
    for ch in line:
        if ch in BARS:
            segs.append(cur); bars.append(ch); cur = ''
        else:
            cur += ch
    tail = cur
    if len(bars) != len(targets):
        raise ValueError(f"bar count {len(bars)} != targets {len(targets)}: {line}")
    out = ''; col = 0
    for seg, bar, tgt in zip(segs, bars, targets):
        need = tgt - col - len(seg)
        if need > 0:
            filler = seg[-1] if seg and seg[-1] in FILL else ' '
            seg = seg + filler * need
        elif need < 0:
            k = -need
            if len(seg) < k or any(c not in FILL for c in seg[-k:]):
                raise ValueError(f"cannot shrink segment {seg!r} by {k} in line: {line}")
            seg = seg[:-k]
        out += seg + bar
        col = tgt + 1
    return out + tail

def main():
    p = sys.argv[1]
    spec = json.loads(sys.argv[2])  # {"148": [0,96], ...}
    lines = open(p, encoding='utf-8').read().split('\n')
    for k, targets in spec.items():
        i = int(k) - 1
        lines[i] = fix_line(lines[i], targets)
    open(p, 'w', encoding='utf-8').write('\n'.join(lines))
    print("fixed", len(spec), "lines")
main()
