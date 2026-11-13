const V = await import(process.argv[2]);
const { contrast, apca } = V;
const toRgb = (c) => { const h = c.replace('#',''); return [0,2,4].map(i => parseInt(h.slice(i,i+2),16)); };
const toHex = (rgb) => '#' + rgb.map(v => Math.round(Math.max(0,Math.min(255,v))).toString(16).padStart(2,'0')).join('');
const mix = (fg, bg, a) => { const f = toRgb(fg), b = toRgb(bg); return toHex(f.map((v,i) => v*a + b[i]*(1-a))); };
const r = (x) => Math.round(x*100)/100;
const show = (label, a, b) => console.log(`${label}: ${a} on ${b} WCAG ${r(contrast(a,b))} APCA ${r(apca(a,b))}`);
// light
show('L accent.strong text on surface', '#256abf', '#fcfcfb');
show('L accent.strong text on window', '#256abf', '#f4f4f1');
show('L white on accent.fill', '#ffffff', '#2a78d6');
show('L white on critical', '#ffffff', '#d03b3b');
show('L ink on warning fill', '#0b0b0b', '#fab219');
show('L ink2 on window', '#52514e', '#f4f4f1');
const Ltrack = mix('#2a78d6', '#ffffff', 0.18); show('L meter fill vs same-ramp track '+Ltrack, '#2a78d6', Ltrack);
const LtrackW = mix('#fab219', '#ffffff', 0.22); show('L warning meter fill vs track '+LtrackW, '#fab219', LtrackW);
const LtrackC = mix('#d03b3b', '#ffffff', 0.18); show('L critical meter fill vs track '+LtrackC, '#d03b3b', LtrackC);
show('L meter track vs raised card', Ltrack, '#ffffff');
// dark
show('D accent.strong text on surface', '#5598e7', '#1a1a19');
show('D accent text on surface', '#3987e5', '#1a1a19');
show('D white on accent.fill', '#ffffff', '#2a78d6');
show('D white on critical', '#ffffff', '#d03b3b');
const Dtrack = mix('#3987e5', '#232321', 0.25); show('D meter fill vs same-ramp track '+Dtrack, '#3987e5', Dtrack);
const DtrackW = mix('#fab219', '#232321', 0.22); show('D warning meter fill vs track '+DtrackW, '#fab219', DtrackW);
const DtrackC = mix('#d03b3b', '#232321', 0.25); show('D critical meter fill vs track '+DtrackC, '#d03b3b', DtrackC);
show('D meter track vs raised card', Dtrack, '#232321');
// selection focus stripe and focus ring on surfaces
show('L focus ring vs surface', '#2a78d6', '#fcfcfb');
show('D focus ring vs surface', '#3987e5', '#1a1a19');
// status icons on surfaces (non-text >= 3:1 desired)
for (const [m,s] of [['L','#fcfcfb'],['D','#1a1a19']]) for (const c of ['#0ca30c','#fab219','#ec835a','#d03b3b']) show(`${m} status ${c} vs surface`, c, s);
// tree row text on dark hover/selected
show('D ink on selected', '#f0efec', '#233b56');
show('L ink on selected', '#0b0b0b', '#dfeaf6');
// treemap selected outline: ink vs base fills
for (const f of ['#2a78d6','#eb6834','#1baf7a','#a8a69e']) show('L outline ink vs fill', '#0b0b0b', f);
for (const f of ['#3987e5','#d95926','#199e70','#5f5e59']) show('D outline ink vs fill', '#f0efec', f);
// scrim composites
console.log('L dialog scrim over surface', mix('#0b0b0b', '#fcfcfb', 0.32), ' D scrim', mix('#000000', '#1a1a19', 0.5));
