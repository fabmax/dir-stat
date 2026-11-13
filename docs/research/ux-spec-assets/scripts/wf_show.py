import sys, unicodedata
p, a, b = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
lines = open(p, encoding='utf-8').read().split('\n')
def cols(s):
    out=[]; c=0
    for ch in s:
        if ch in '│┬┴┼├┤╮╯╭╰': out.append(c)
        c += 1
    return out, c
for i in range(a, b+1):
    t = lines[i-1]
    cs, width = cols(t)
    print(f"{i:4d} w={width:3d} bars={cs}")
