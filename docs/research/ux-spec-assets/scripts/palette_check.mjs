// Computes blended treemap steps, UI token composites, and WCAG/APCA contrasts
// for the dir-stat UX spec. Imports the dataviz skill validator by absolute path.
const V = await import(process.argv[2]);
const { contrast, apca, validate } = V;

const hex = (c) => c.replace('#','');
const toRgb = (c) => { const h = hex(c); return [0,2,4].map(i => parseInt(h.slice(i,i+2),16)); };
const toHex = (rgb) => '#' + rgb.map(v => Math.round(Math.max(0,Math.min(255,v))).toString(16).padStart(2,'0')).join('');
// sRGB-space alpha blend (what a GPU does for an alpha-composited quad without linear blending)
const mix = (fg, bg, a) => { const f = toRgb(fg), b = toRgb(bg); return toHex(f.map((v,i) => v*a + b[i]*(1-a))); };
const r2 = (x) => Math.round(x*100)/100;

const themes = {
  light: { page:'#f4f4f1', surface:'#fcfcfb', raised:'#ffffff', hair:'#e1e0d9', base:'#c3c2b7',
           ink:'#0b0b0b', ink2:'#52514e', muted:'#898781', accent:'#2a78d6',
           slots:['#2a78d6','#eb6834','#1baf7a'], other:'#a8a69e',
           good:'#0ca30c', warn:'#fab219', serious:'#ec835a', crit:'#d03b3b' },
  dark:  { page:'#141413', surface:'#1a1a19', raised:'#232321', hair:'#2c2c2a', base:'#383835',
           ink:'#f0efec', ink2:'#c3c2b7', muted:'#898781', accent:'#3987e5',
           slots:['#3987e5','#d95926','#199e70'], other:'#5f5e59',
           good:'#0ca30c', warn:'#fab219', serious:'#ec835a', crit:'#d03b3b' },
};

for (const [mode, t] of Object.entries(themes)) {
  console.log(`\n===== ${mode} =====`);
  const hover = mode==='light' ? mix('#0b0b0b', t.surface, 0.04) : mix('#ffffff', t.surface, 0.05);
  const sel = mix(t.accent, t.surface, mode==='light' ? 0.14 : 0.30);
  const selUnfocused = mix(mode==='light' ? '#0b0b0b' : '#ffffff', t.surface, mode==='light'?0.07:0.09);
  const track = mode==='light' ? mix('#0b0b0b', t.surface, 0.07) : mix('#ffffff', t.surface, 0.08);
  console.log('hover', hover, ' selected', sel, ' selected-unfocused', selUnfocused, ' bar-track', track);
  // Text contrast on the backgrounds text actually sits on
  for (const bgName of ['page','surface','raised']) {
    const bg = t[bgName];
    console.log(`text on ${bgName} ${bg}: ink ${r2(contrast(t.ink,bg))}  ink2 ${r2(contrast(t.ink2,bg))}  muted ${r2(contrast(t.muted,bg))}  accent ${r2(contrast(t.accent,bg))}`);
  }
  console.log(`text on selected ${sel}: ink ${r2(contrast(t.ink,sel))} ink2 ${r2(contrast(t.ink2,sel))} muted ${r2(contrast(t.muted,sel))}`);
  console.log(`text on hover ${hover}: ink2 ${r2(contrast(t.ink2,hover))}`);
  console.log(`bar fill ${t.accent} vs surface ${r2(contrast(t.accent,t.surface))}, vs track ${r2(contrast(t.accent,track))}, vs selected ${r2(contrast(t.accent,sel))}, track vs surface ${r2(contrast(track,t.surface))}`);
  // Treemap level steps: level1 = base slot, level2/3 = blended toward surface
  const levels = [1.0, 0.72, 0.52];
  const pick = (fill) => { const w = contrast('#ffffff', fill), k = contrast('#0b0b0b', fill); return w >= k ? ['#ffffff', w] : ['#0b0b0b', k]; };
  for (const s of [...t.slots, t.other]) {
    const row = levels.map(a => { const f = mix(s, t.surface, a); const [lab, c] = pick(f); return `${f} (label ${lab} ${r2(c)}:1, vs surface ${r2(contrast(f,t.surface))})`; });
    console.log(`slot ${s}: ` + row.join(' | '));
  }
  // status
  for (const k of ['good','warn','serious','crit']) {
    const c = t[k];
    console.log(`status ${k} ${c}: vs surface ${r2(contrast(c,t.surface))}; chip bg ${mix(c,t.surface,0.14)} ink on chip ${r2(contrast(t.ink, mix(c,t.surface,0.14)))} ink2 on chip ${r2(contrast(t.ink2, mix(c,t.surface,0.14)))}`);
  }
}
