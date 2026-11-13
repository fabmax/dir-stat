const V = await import(process.argv[2]);
const { contrast, apca } = V;
const r = (x) => Math.round(x*100)/100;
for (const f of ['#2a78d6','#256abf','#1c5cab','#184f95']) console.log(`white on ${f}: WCAG ${r(contrast('#ffffff',f))} APCA ${r(apca('#ffffff',f))}; vs light surface ${r(contrast(f,'#fcfcfb'))}; vs dark surface ${r(contrast(f,'#1a1a19'))}`);
