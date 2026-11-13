const V = await import(process.argv[2]);
const { contrast, apca } = V;
const toRgb = (c) => { const h = c.replace('#',''); return [0,2,4].map(i => parseInt(h.slice(i,i+2),16)); };
const toHex = (rgb) => '#' + rgb.map(v => Math.round(Math.max(0,Math.min(255,v))).toString(16).padStart(2,'0')).join('');
const mix = (fg, bg, a) => { const f = toRgb(fg), b = toRgb(bg); return toHex(f.map((v,i) => v*a + b[i]*(1-a))); };
const r1 = (x) => Math.round(x*10)/10;
const sets = {
  light: { surface:'#fcfcfb', fills:['#2a78d6','#eb6834','#1baf7a','#a8a69e','#0ca30c','#fab219'] },
  dark:  { surface:'#1a1a19', fills:['#3987e5','#d95926','#199e70','#5f5e59','#0ca30c','#fab219'] },
};
for (const [mode, s] of Object.entries(sets)) {
  console.log(`== ${mode}`);
  for (const f of s.fills) {
    for (const a of [1.0, 0.72, 0.52]) {
      const fill = mix(f, s.surface, a);
      const w = apca('#ffffff', fill), k = apca('#0b0b0b', fill);
      const best = Math.abs(w) >= Math.abs(k) ? 'white' : 'ink';
      console.log(`${f}@${a} -> ${fill}: APCA white ${r1(w)} ink ${r1(k)} => ${best}; WCAG white ${r1(contrast('#ffffff',fill))} ink ${r1(contrast('#0b0b0b',fill))}`);
    }
  }
}
