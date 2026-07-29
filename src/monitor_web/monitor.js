const state = { cutoffTimeMs: 0, cutoffMessageCount: 0, stabilityCutoffTimeMs: 0, latestData: null };
const storageKey = "fp10-monitor-settings";
const bands = [
  {value:20,label:"pp"},
  {value:40,label:"p"},
  {value:60,label:"mp"},
  {value:80,label:"mf"},
  {value:100,label:"f"},
  {value:120,label:"ff"},
];
const dynamicZones = [
  {max:27,label:"pp",className:"pp"},
  {max:30,label:"pp/p",className:"pp-p"},
  {max:47,label:"p",className:"p"},
  {max:50,label:"p/mp",className:"p-mp"},
  {max:67,label:"mp",className:"mp"},
  {max:70,label:"mp/mf",className:"mp-mf"},
  {max:87,label:"mf",className:"mf"},
  {max:90,label:"mf/f",className:"mf-f"},
  {max:107,label:"f",className:"f"},
  {max:110,label:"f/ff",className:"f-ff"},
  {max:127,label:"ff",className:"ff"},
];
const workingRanges = [
  {min:12,max:30,label:"pp",className:"pp"},
  {min:28,max:50,label:"p",className:"p"},
  {min:48,max:70,label:"mp",className:"mp"},
  {min:68,max:90,label:"mf",className:"mf"},
  {min:88,max:110,label:"f",className:"f"},
  {min:108,max:127,label:"ff",className:"ff"},
];
const $ = (id) => document.getElementById(id);
const filterLabels = {all:"全部音区", left:"左手", right:"右手"};
function loadSettings(){ try{ return JSON.parse(localStorage.getItem(storageKey) || "{}") || {}; } catch(e){ return {}; } }
function saveSettings(settings){ try{ localStorage.setItem(storageKey, JSON.stringify(settings)); } catch(e){} }
function updateSettings(patch){ saveSettings({...loadSettings(), ...patch}); }
function restoreSettings(){
  const settings=loadSettings();
  if(settings.key && [...$("keySelect").options].some(option=>option.value===settings.key)) $("keySelect").value=settings.key;
  applyFilterSettings(settings.handFilter || "all", Number(settings.splitNote || 60));
}
function currentFilterSettings(){
  const settings=loadSettings();
  const mode=["all","left","right"].includes(settings.handFilter) ? settings.handFilter : "all";
  const split=Number(settings.splitNote || 60);
  return {mode,split:Number.isFinite(split) ? split : 60};
}
function applyFilterSettings(mode, split){
  document.querySelectorAll(".filter-button").forEach(button=>button.classList.toggle("active", button.dataset.handFilter===mode));
  document.querySelectorAll(".split-select").forEach(select=>{ select.value=String(split); });
  $("mobileFilterSummary").textContent = mode==="all" ? "全部音区" : `${filterLabels[mode]} · ${noteName(split)} ${mode==="right" ? "及以上" : "以下"}`;
}
function filterSummaryText(filter=currentFilterSettings()){
  if(filter.mode==="all") return "全部音区";
  return `${filterLabels[filter.mode]} ${noteName(filter.split)} ${filter.mode==="right" ? "及以上" : "以下"}`;
}
function noteName(note){
  const names=["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
  return `${names[note%12]}${Math.floor(note/12)-1}`;
}
function notePassesFilter(note, filter=currentFilterSettings()){
  if(filter.mode==="right") return note.note >= filter.split;
  if(filter.mode==="left") return note.note < filter.split;
  return true;
}
function recomputeGroup(group, notes){
  if(!group || !notes.length) return null;
  const velocities=notes.map(note=>note.velocity);
  const min=Math.min(...velocities);
  const max=Math.max(...velocities);
  const average=Math.round(velocities.reduce((sum,value)=>sum+value,0)/velocities.length);
  return {...group, notes, average, spread:max-min};
}
function filterGroup(group, filter=currentFilterSettings()){
  if(!group) return null;
  return recomputeGroup(group, group.notes.filter(note=>notePassesFilter(note, filter)));
}
function filterHistory(history, filter=currentFilterSettings()){
  return history.map(group=>filterGroup(group, filter)).filter(Boolean);
}
function dynamicForVelocity(v){ return dynamicZones.find(zone=>v<=zone.max) || dynamicZones[dynamicZones.length-1]; }
function cls(v){ return `velocity-${dynamicForVelocity(v).className}`; }
function color(v){ return getComputedStyle(document.documentElement).getPropertyValue(`--${cls(v).replace("velocity-","")}`).trim(); }
function markForVelocity(v){ return v > 0 ? dynamicForVelocity(v).label : "-"; }
function notePos(note){ const names=["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"]; const isBlack=(n)=>names[n%12].includes("#"); let count=0; for(let n=21;n<Math.max(21,Math.min(108,note));n++) if(!isBlack(n)) count++; const center=isBlack(note)?count:count+.5; return center/52*100; }
function time(ms){ return new Date(Number(ms)).toLocaleTimeString("zh-CN",{hour12:false,hour:"2-digit",minute:"2-digit",second:"2-digit",fractionalSecondDigits:2}); }
function spell(note){ return spellingFor(note).name; }
function noteLabel(note){ return spell(note.note); }
function spellingFor(note){
  const pc=note%12; const octave=Math.floor(note/12)-1; const info=keyInfo();
  if(info.scale[pc]){
    const s=info.scale[pc];
    return {letter:s.letter,letterIndex:s.index,octave,accidental:"",name:`${s.letter}${s.keyAccidental||""}${octave}`};
  }
  const naturals={0:["C",0],2:["D",1],4:["E",2],5:["F",3],7:["G",4],9:["A",5],11:["B",6]};
  if(naturals[pc]){
    const [letter,index]=naturals[pc];
    return {letter,letterIndex:index,octave,accidental:"♮",name:`${letter}${octave}`};
  }
  const sharp={1:["C",0],3:["D",1],6:["F",3],8:["G",4],10:["A",5]};
  const flat={1:["D",1],3:["E",2],6:["G",4],8:["A",5],10:["B",6]};
  const [letter,index]=(info.prefer==="flat"?flat:sharp)[pc];
  const accidental=info.prefer==="flat"?"♭":"♯";
  return {letter,letterIndex:index,octave,accidental,name:`${letter}${accidental}${octave}`};
}
function render(data){
  state.latestData = data;
  $("status").textContent = `${data.inputName} · ${data.messageCount} messages`;
  const filter=currentFilterSettings();
  applyFilterSettings(filter.mode, filter.split);
  const history = visibleHistory(filterHistory(data.history || [], filter));
  const currentChord = filterGroup(data.currentChord, filter);
  const last = recentNotes(history, 1)[0] || null;
  const raw = (data.raw || []).filter(e=>Number(e.timeMs)>state.cutoffTimeMs);
  if(last && history.length){ $("lastName").textContent=noteLabel(last); $("lastVelocity").textContent=last.velocity; $("lastBar").style.width=`${last.velocity/127*100}%`; $("lastBar").className=`fill ${cls(last.velocity)}`; $("lastMark").textContent=last.mark; }
  else { $("lastName").textContent="-"; $("lastVelocity").textContent="0"; $("lastBar").style.width="0%"; $("lastMark").textContent="-"; }
      renderChord(currentChord);
      renderTimeline(history);
      renderHistory(history);
      renderRaw(raw, Math.max(0, data.messageCount - state.cutoffMessageCount));
      renderMobile({...data,lastNote:last,currentChord}, history, raw);
    }
function visibleHistory(history){ return history.filter(group=>Number(group.timeMs)>state.cutoffTimeMs); }
function renderChord(group){
  const keyboard=$("keyboard"); const cards=$("cards"); keyboard.innerHTML=""; cards.innerHTML="";
  const chordGraphHeight = 160;
  renderDynamicZones(keyboard, chordGraphHeight);
  for(const band of bands){ const g=document.createElement("div"); g.className="guide"; g.style.bottom=`${band.value/127*chordGraphHeight}px`; g.innerHTML=`<span>${band.label}</span><b>${band.value}</b>`; keyboard.append(g); }
  if(!group){ $("chordSummary").textContent="等待输入"; return; }
  $("chordSummary").textContent=`${group.notes.length} 音 · 平均 ${group.average} · 差距 ${group.spread}`;
  for(const note of group.notes){ const name=noteLabel(note); const marker=document.createElement("div"); marker.className=`key ${cls(note.velocity)}`; marker.style.left=`${notePos(note.note)}%`; marker.style.height=`${Math.max(2,note.velocity/127*chordGraphHeight)}px`; marker.innerHTML=`<span class="value">${name} ${note.velocity}</span>`; keyboard.append(marker);
    const card=document.createElement("article"); card.className="card"; card.innerHTML=`<strong>${name}</strong><div class="mini"><span>力度</span><div class="track"><div class="fill ${cls(note.velocity)}" style="width:${note.velocity/127*100}%"></div></div><b>${note.velocity}</b></div>`; cards.append(card); }
}
function renderTimeline(history){
  const timeline=$("timeline"); const groups=visibleGroups(history); timeline.innerHTML="";
  if(!groups.length){ timeline.innerHTML=`<div class="timeline-empty">等待输入</div>`; return; }
  const span=(groups[groups.length-1].timeMs-groups[0].timeMs)/1000;
  $("timelineSummary").textContent = `${filterSummaryText()} · ${$("keySelect").value} 调 · ${groups.length} 组 · ${span.toFixed(1)}s · 上方音高，下方力度`;
  renderStaffPlot(timeline, groups);
}
function visibleGroups(history){
  const ordered=history.slice(0,48).reverse();
  if(ordered.length<2) return ordered;
  const latest=ordered[ordered.length-1].timeMs;
  const earliest=ordered[0].timeMs;
  const fullSpan=Math.max(1,latest-earliest);
  const targetWindow=Math.min(Math.max(fullSpan,4500),22000);
  const start=latest-targetWindow;
  return ordered.filter(group=>group.timeMs>=start);
}
function timeScale(groups){
  const left=12, right=4;
  if(groups.length<=1) return {left,right,start:0,span:1};
  const latest=groups[groups.length-1].timeMs;
  const earliest=groups[0].timeMs;
  const span=Math.max(4500,latest-earliest);
  return {left,right,start:latest-span,span};
}
function xForTime(group,scale){ return scale.left + ((group.timeMs-scale.start)/scale.span) * (100-scale.left-scale.right); }
function add(el, clsName, style, text){ const node=document.createElement("div"); node.className=clsName; Object.assign(node.style, style); if(text!==undefined) node.textContent=text; el.append(node); return node; }
function renderDynamicZones(el, height, bottom=0){
  for(const range of workingRanges){
    const zone=add(el,`dynamic-zone velocity-${range.className}`,{
      bottom:`${bottom + range.min/127*height}px`,
      height:`${(range.max-range.min)/127*height}px`,
    });
    zone.title=`${range.label} ${range.min}-${range.max}`;
  }
}
function drawRuler(el, scale){
  const seconds=Math.ceil(scale.span/1000); const step=seconds>12?2:1;
  for(let s=0;s<=seconds;s+=step){
    const ms=scale.start+s*1000; const x=scale.left+((ms-scale.start)/scale.span)*(100-scale.left-scale.right);
    if(x<scale.left-.1 || x>100-scale.right+.1) continue;
    add(el,"ruler-tick",{left:`${x}%`});
    add(el,"ruler-label",{left:`${x}%`},`${Math.round((ms-(scale.start+scale.span))/1000)}s`);
  }
}
function renderStaffPlot(el, groups){
  const scale=timeScale(groups);
  drawRuler(el, scale);
  [62,74,86,98,110,134,146,158,170,182].forEach(y=>add(el,"staff-line",{top:`${y}px`}));
  add(el,"staff-middle",{top:"122px"});
  add(el,"staff-clef treble",{},"𝄞");
  add(el,"staff-clef bass",{},"𝄢");
  renderKeySignature(el);
  add(el,"velocity-lane",{});
  renderVelocityGrid(el);
  add(el,"velocity-title",{},"velocity");
  const accidentalMemory = new Map();
  let eventIndex = 0;
  groups.forEach((group,index)=>{
    const x=xForTime(group,scale); const latest=index===groups.length-1;
    group.notes.forEach((note,noteIndex)=>{
      const spelling=spellingFor(note.note);
      const display=displayStaffPosition(spelling);
      const y=display.y; const dx=(noteIndex-(group.notes.length-1)/2)*9; const c=color(note.velocity);
      for(const lineY of ledgerLines(y)){ add(el,"staff-ledger",{left:`calc(${x}% + ${dx}px)`,top:`${lineY}px`}); }
      const marker=add(el,`plot-note ${latest?"latest":""}`,{left:`calc(${x}% + ${dx}px)`,top:`${y}px`,background:c},"");
      marker.title=`${spelling.name}${display.mark ? " · " + display.mark : ""} · ${note.velocity}`;
      const accidental=accidentalForSpelling(accidentalMemory, spelling, group.timeMs, eventIndex);
      if(accidental) add(el,"accidental",{left:`calc(${x}% + ${dx - noteIndex * 5}px)`,top:`${y}px`},accidental);
      if(display.mark) add(el,"octave-mark",{left:`calc(${x}% + ${dx}px)`,top:`${display.markY}px`},display.mark);
      add(el,"velocity-bar",{left:`calc(${x}% + ${dx}px)`,height:`${Math.max(3,note.velocity/127*82)}px`,background:c}).title=`${spelling.name} · ${note.velocity}`;
      eventIndex += 1;
    });
  });
}
function renderVelocityGrid(el){
  const laneBottom=38, laneHeight=82;
  renderDynamicZones(el, laneHeight, laneBottom);
  for(const value of [0,...bands.map(band=>band.value),127]){
    const bottom=laneBottom + value/127*laneHeight;
    add(el,"velocity-grid-line",{bottom:`${bottom}px`});
    add(el,"velocity-grid-label",{bottom:`${bottom}px`},String(value));
  }
}
function staffStepFromSpelling(spelling){ return spelling.octave*7+spelling.letterIndex; }
function staffYFromSpelling(spelling){ return 122-(staffStepFromSpelling(spelling)-28)*6; }
function staffY(note){ return staffYFromSpelling(spellingFor(note)); }
function displayStaffPosition(spelling){
  let y=staffYFromSpelling(spelling); let octaveShift=0;
  while(y<26){ y+=42; octaveShift+=1; }
  while(y>224){ y-=42; octaveShift-=1; }
  const mark=octaveShift===0 ? "" : octaveShift===1 ? "8va" : octaveShift>=2 ? "15ma" : octaveShift===-1 ? "8vb" : "15mb";
  return {y,mark,markY:octaveShift>0?Math.max(14,y-18):Math.min(232,y+18)};
}
function ledgerLines(y){
  const lines=[];
  if(Math.abs(y-122)<1) lines.push(122);
  if(y<62){ for(let lineY=50; lineY>=Math.max(26,y-1); lineY-=12) lines.push(lineY); }
  if(y>182){ for(let lineY=194; lineY<=Math.min(224,y+1); lineY+=12) lines.push(lineY); }
  return lines;
}
function keyInfo(){
  const natural={0:["C",0,""],2:["D",1,""],4:["E",2,""],5:["F",3,""],7:["G",4,""],9:["A",5,""],11:["B",6,""]};
  const configs={
    C:{prefer:"sharp",alter:{}},
    G:{prefer:"sharp",alter:{6:["F",3,"♯"]}},
    D:{prefer:"sharp",alter:{6:["F",3,"♯"],1:["C",0,"♯"]}},
    A:{prefer:"sharp",alter:{6:["F",3,"♯"],1:["C",0,"♯"],8:["G",4,"♯"]}},
    E:{prefer:"sharp",alter:{6:["F",3,"♯"],1:["C",0,"♯"],8:["G",4,"♯"],3:["D",1,"♯"]}},
    F:{prefer:"flat",alter:{10:["B",6,"♭"]}},
    Bb:{prefer:"flat",alter:{10:["B",6,"♭"],3:["E",2,"♭"]}},
    Eb:{prefer:"flat",alter:{10:["B",6,"♭"],3:["E",2,"♭"],8:["A",5,"♭"]}}
  };
  const config=configs[$("keySelect").value] || configs.C;
  const scale={};
  for(const [pc,parts] of Object.entries(natural)){
    scale[pc]={letter:parts[0],index:parts[1],keyAccidental:parts[2]};
  }
  for(const [pc,parts] of Object.entries(config.alter)){
    scale[pc]={letter:parts[0],index:parts[1],keyAccidental:parts[2]};
    for(const naturalPc of Object.keys(scale)){
      if(scale[naturalPc].letter===parts[0] && naturalPc!==pc) delete scale[naturalPc];
    }
  }
  return {prefer:config.prefer,scale};
}
function renderKeySignature(el){
  for(const item of keySignatureMarks()){
    add(el,"key-sig",{left:`${55 + item.index * 8}px`,top:`${staffY(item.treble)}px`},item.symbol);
    add(el,"key-sig",{left:`${55 + item.index * 8}px`,top:`${staffY(item.bass)}px`},item.symbol);
  }
}
function keySignatureMarks(){
  const key=$("keySelect").value;
  const sharps=[
    {treble:78,bass:42},
    {treble:73,bass:37},
    {treble:80,bass:44},
    {treble:75,bass:39}
  ];
  const flats=[
    {treble:70,bass:46},
    {treble:75,bass:51},
    {treble:68,bass:44}
  ];
  const sharpCount={G:1,D:2,A:3,E:4}[key]||0;
  const flatCount={F:1,Bb:2,Eb:3}[key]||0;
  if(sharpCount) return sharps.slice(0,sharpCount).map((mark,index)=>({...mark,index,symbol:"♯"}));
  if(flatCount) return flats.slice(0,flatCount).map((mark,index)=>({...mark,index,symbol:"♭"}));
  return [];
}
function accidentalForSpelling(memory, spelling, timeMs, eventIndex){
  const key=spelling.letter;
  const last=memory.get(key);
  const accidental=spelling.accidental;
  let show=false;
  if(accidental){
    show=!last || last.accidental!==accidental || timeMs-last.timeMs>1100 || eventIndex-last.index>4;
  } else if(last && last.accidental && (timeMs-last.timeMs<2200 || eventIndex-last.index<=6)) {
    show=true;
  }
  memory.set(key,{accidental,timeMs,index:eventIndex});
  return show ? (accidental || "♮") : "";
}
function renderHistory(history){
  $("historySummary").textContent=`${history.length} groups`;
  $("history").innerHTML = history.map((group,index)=>`<tr class="group-row"><td colspan="4"><div class="group-meta"><strong>#${history.length-index}</strong><span>${time(group.timeMs)}</span><span>${group.notes.length} 音 · ${group.notes.map(noteLabel).join(" ")}</span><span>平均 ${group.average}</span><span>差距 ${group.spread}</span></div></td></tr>${group.notes.map(note=>`<tr><td></td><td><span class="pill">${noteLabel(note)}</span></td><td><div class="history-meter"><span class="${cls(note.velocity)}" style="width:${note.velocity/127*100}%"></span><b>${note.velocity}</b></div></td><td>${note.mark}</td></tr>`).join("")}`).join("");
}
    function renderRaw(raw,count){ $("rawSummary").textContent=`${count} messages`; $("raw").innerHTML=raw.map(e=>`<div class="raw-row"><span>${time(e.timeMs)}</span><strong>${e.kind}</strong><span>ch ${e.channel}</span><code>${e.bytes}</code></div>`).join(""); }
    function recentNotes(history, limit=32, afterMs=0){
      const notes=[];
      for(const group of history){
        if(Number(group.timeMs)<=afterMs) continue;
        for(const note of group.notes) notes.push({...note,timeMs:group.timeMs});
        if(notes.length>=limit) break;
      }
      return notes.slice(0,limit);
    }
    function velocityStats(notes){
      if(!notes.length) return {count:0,average:0,min:0,max:0,spread:0,sd:0,score:0};
      const velocities=notes.map(note=>note.velocity);
      const sum=velocities.reduce((total,value)=>total+value,0);
      const average=Math.round(sum/velocities.length);
      const min=Math.min(...velocities);
      const max=Math.max(...velocities);
      const variance=velocities.reduce((total,value)=>total+Math.pow(value-average,2),0)/velocities.length;
      const sd=Math.sqrt(variance);
      return {count:velocities.length,average,min,max,spread:max-min,sd,score:Math.max(0,Math.min(100,Math.round(100-sd*3)))};
    }
    function chordVelocityRange(group){
      if(!group || !group.notes.length) return null;
      const velocities=group.notes.map(note=>note.velocity);
      return {min:Math.min(...velocities),max:Math.max(...velocities),spread:group.spread,average:group.average};
    }
    function feedbackFor(stats,group,last){
      const range=chordVelocityRange(group);
      if(range && group.notes.length>1 && range.spread>=18) return {title:"和弦不均",detail:`低 ${range.min} · 高 ${range.max} · 差距 ${range.spread}`};
      if(range && group.notes.length>1 && range.spread<=8) return {title:"和弦均匀",detail:`平均 ${range.average} · 差距 ${range.spread}`};
      if(!last || !stats.count) return {title:"等待输入",detail:"弹几个音后开始分析"};
      if(stats.count<4) return {title:"继续弹",detail:"再弹几下会出现稳定度"};
      if(stats.sd<=4) return {title:"很稳定",detail:`${markForVelocity(stats.average)} · 平均 ${stats.average} · 波动 ±${Math.round(stats.sd)}`};
      if(stats.sd<=8) return {title:"基本稳定",detail:`${markForVelocity(stats.average)} · 平均 ${stats.average} · 波动 ±${Math.round(stats.sd)}`};
      return {title:"力度起伏大",detail:`${markForVelocity(stats.average)} · 平均 ${stats.average} · 范围 ${stats.min}-${stats.max}`};
    }
    function renderMobileHeroMeter(group,last){
      const range=chordVelocityRange(group);
      if(range && group.notes.length>1){
        $("mobileDynamic").textContent=`${range.min}-${range.max}`;
        $("mobileDynamic").style.color="";
        $("mobileVelocity").textContent=`差距 ${range.spread}`;
      } else if(last) {
        $("mobileDynamic").textContent=String(last.velocity);
        $("mobileDynamic").style.color=color(last.velocity);
        $("mobileVelocity").textContent=last.mark;
      } else {
        $("mobileDynamic").textContent="-";
        $("mobileDynamic").style.color="";
        $("mobileVelocity").textContent="0";
      }
    }
    function renderMobile(data, history, raw){
      const last=data.lastNote;
      const notes=recentNotes(history, 32, state.stabilityCutoffTimeMs);
      const stats=velocityStats(notes);
      const feedback=feedbackFor(stats,data.currentChord,last);
      $("mobileFeedback").textContent=feedback.title;
      $("mobileFeedbackDetail").textContent=feedback.detail;
      renderMobileHeroMeter(data.currentChord,last);
      $("mobileStabilityScore").textContent=stats.count ? `${stats.score}%` : "--";
      $("mobileStabilitySummary").textContent=stats.count ? `${stats.count} 个音 · 平均 ${stats.average} · 范围 ${stats.min}-${stats.max}` : "等待输入";
      $("mobileStabilityBar").style.width=`${stats.score}%`;
      renderMobileTrend(history);
      renderMobileChord(data.currentChord);
      $("mobileDetailHistory").textContent=`${history.length} groups`;
      $("mobileDetailRaw").textContent=`${Math.max(0, data.messageCount - state.cutoffMessageCount)} messages`;
    }
    function renderMobileTrend(history){
      const el=$("mobileTrend");
      const groups=visibleGroups(history).slice(-24);
      if(!groups.length){ el.innerHTML=`<div class="mobile-empty">等待输入</div>`; return; }
      const values=groups.map(group=>group.average);
      const min=Math.min(...values);
      const max=Math.max(...values);
      $("mobileTrendSummary").textContent=`${groups.length} 组 · 平均 ${Math.round(values.reduce((sum,value)=>sum+value,0)/values.length)} · 范围 ${min}-${max}`;
      el.innerHTML="";
      renderDynamicZones(el, 92, 12);
      el.insertAdjacentHTML("beforeend", groups.map(group=>`<span class="mobile-trend-bar ${cls(group.average)}" title="${time(group.timeMs)} · ${group.average}" style="height:${Math.max(8,Math.round(group.average/127*100))}%"></span>`).join(""));
    }
    function renderMobileChord(group){
      const el=$("mobileChord");
      if(!group){ $("mobileChordSummary").textContent="等待输入"; $("mobileChordScore").textContent="--"; el.innerHTML=`<div class="mobile-empty">等待输入</div>`; return; }
      const score=Math.max(0,Math.min(100,100-Math.round(group.spread*2.2)));
      $("mobileChordSummary").textContent=`从低到高 · 平均 ${group.average} · 差距 ${group.spread}`;
      $("mobileChordScore").textContent=`${score}%`;
      el.innerHTML=group.notes.map(note=>{
        const name=noteLabel(note);
        return `<div class="mobile-note" title="${name} · ${note.mark} · ${note.velocity}"><div class="mobile-note-track"><span class="${cls(note.velocity)}" style="height:${Math.max(6,Math.round(note.velocity/127*100))}%"></span></div><strong>${name}</strong><em>${note.mark}</em><b>${note.velocity}</b></div>`;
      }).join("");
    }
    $("clearView").addEventListener("click",async()=>{ try{ const data=await fetch("/state",{cache:"no-store"}).then(r=>r.json()); state.cutoffTimeMs=Date.now(); state.stabilityCutoffTimeMs=state.cutoffTimeMs; state.cutoffMessageCount=data.messageCount||0; render({...data,history:[],raw:[],lastNote:null,currentChord:null}); } catch(e){ state.cutoffTimeMs=Date.now(); state.stabilityCutoffTimeMs=state.cutoffTimeMs; } });
$("mobileClearStability").addEventListener("click",()=>{ state.stabilityCutoffTimeMs=Date.now(); if(state.latestData) render(state.latestData); });
document.querySelectorAll(".filter-button").forEach(button=>button.addEventListener("click",()=>{
  const split=currentFilterSettings().split;
  updateSettings({handFilter:button.dataset.handFilter, splitNote:split});
  if(state.latestData) render(state.latestData); else applyFilterSettings(button.dataset.handFilter, split);
}));
document.querySelectorAll(".split-select").forEach(select=>select.addEventListener("change",()=>{
  const mode=currentFilterSettings().mode;
  const split=Number(select.value || 60);
  updateSettings({handFilter:mode, splitNote:split});
  if(state.latestData) render(state.latestData); else applyFilterSettings(mode, split);
}));
$("seedLog").addEventListener("click",async()=>{ const button=$("seedLog"); button.disabled=true; button.textContent="载入中"; try{ const result=await fetch("/seed-log",{cache:"no-store"}).then(r=>r.json()); button.textContent=result.loaded ? `已载入 ${result.loaded}` : "无日志"; } catch(e){ button.textContent="载入失败"; } setTimeout(()=>{ button.disabled=false; button.textContent="载入日志"; },1400); });
let metronomeState = null;
let metronomeServerOffsetMs = 0;
let metronomeAnimationFrame = null;
let metronomeFallbackTimer = null;
let metronomeEventSource = null;
function metronomeControls(selector){ return Array.from(document.querySelectorAll(selector)); }
function renderMetronome(metro){
  metronomeState = metro;
  if(Number.isFinite(Number(metro.serverTimeMs))) metronomeServerOffsetMs = Number(metro.serverTimeMs) - Date.now();
  metronomeControls(".metronome-toggle").forEach(button=>{
    button.textContent = metro.running ? "Stop" : "Start";
    button.classList.toggle("running", !!metro.running);
  });
  metronomeControls(".metronome-bpm").forEach(input=>{ if(document.activeElement!==input) input.value=metro.bpm; });
  metronomeControls(".metronome-beats").forEach(select=>{ select.value=String(metro.beatsPerBar || 4); });
  metronomeControls(".metronome-subdivision").forEach(select=>{ select.value=String(metro.subdivision || 1); });
  metronomeControls(".metronome-volume").forEach(input=>{ input.value=metro.volume ?? 0.35; });
  metronomeControls(".metronome-output").forEach(select=>{
    const selected = metro.outputDevice || "";
    const options = [`<option value="">Default output</option>`].concat((metro.devices || []).map(device=>`<option value="${escapeAttr(device)}">${escapeHtml(device)}</option>`));
    const html = options.join("");
    if(select.innerHTML !== html) select.innerHTML = html;
    select.value = selected;
  });
  const deviceLabel = metro.outputDevice || metro.activeDevice || "Default output";
  document.querySelectorAll('[data-metronome-field="device"]').forEach(el=>{ el.textContent = deviceLabel; });
  document.querySelectorAll('[data-metronome-field="error"]').forEach(el=>{ el.textContent = metro.error || ""; });
  renderMetronomeBeat();
  scheduleMetronomeAnimation();
}
function renderMetronomeBeat(){
  const metro = metronomeState || {};
  const beat = currentMetronomeBeat(metro);
  metronomeControls(".metronome-beats-display").forEach(el=>{
    const beats = Math.max(1, Number(metro.beatsPerBar || 4));
    el.innerHTML = Array.from({length:beats},(_,index)=>{
      const active = metro.running && index === beat;
      return `<span class="metronome-dot ${index===0?"accent":""} ${active?"active":""}"></span>`;
    }).join("");
  });
}
function currentMetronomeBeat(metro){
  if(!metro.running) return Number(metro.currentBeat || 0);
  const startedAt = Number(metro.startedAtMs);
  if(!Number.isFinite(startedAt)) return Number(metro.currentBeat || 0);
  const bpm = Math.max(1, Number(metro.bpm || 80));
  const subdivision = Math.max(1, Number(metro.subdivision || 1));
  const beats = Math.max(1, Number(metro.beatsPerBar || 4));
  const elapsed = Math.max(0, Date.now() + metronomeServerOffsetMs - startedAt);
  const clickMs = 60000 / bpm / subdivision;
  const clickIndex = Math.floor(elapsed / clickMs);
  return Math.floor(clickIndex / subdivision) % beats;
}
function scheduleMetronomeAnimation(){
  if(metronomeAnimationFrame || !metronomeState?.running) return;
  const tick=()=>{
    metronomeAnimationFrame = null;
    if(!metronomeState?.running) return;
    renderMetronomeBeat();
    metronomeAnimationFrame = requestAnimationFrame(tick);
  };
  metronomeAnimationFrame = requestAnimationFrame(tick);
}
function escapeHtml(value){ return String(value).replace(/[&<>"']/g,ch=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[ch])); }
function escapeAttr(value){ return escapeHtml(value); }
async function fetchMetronome(){
  try{ renderMetronome(await fetch("/metronome",{cache:"no-store"}).then(r=>r.json())); }
  catch(e){ document.querySelectorAll('[data-metronome-field="error"]').forEach(el=>{ el.textContent = "Metronome offline"; }); }
}
async function updateMetronome(patch){
  const params = new URLSearchParams();
  Object.entries(patch).forEach(([key,value])=>params.set(key, value));
  try{ renderMetronome(await fetch(`/metronome?${params.toString()}`,{cache:"no-store"}).then(r=>r.json())); }
  catch(e){ document.querySelectorAll('[data-metronome-field="error"]').forEach(el=>{ el.textContent = "Metronome update failed"; }); }
}
function startMetronomeFallback(){
  if(metronomeFallbackTimer) return;
  metronomeFallbackTimer = setInterval(fetchMetronome, 2000);
}
function stopMetronomeFallback(){
  if(!metronomeFallbackTimer) return;
  clearInterval(metronomeFallbackTimer);
  metronomeFallbackTimer = null;
}
function connectMetronomeEvents(){
  if(!window.EventSource){ startMetronomeFallback(); fetchMetronome(); return; }
  metronomeEventSource = new EventSource("/metronome-events");
  metronomeEventSource.addEventListener("metronome",(event)=>{ try{ renderMetronome(JSON.parse(event.data)); } catch(e){} });
  metronomeEventSource.onerror = ()=>{
    if(metronomeEventSource){ metronomeEventSource.close(); metronomeEventSource=null; }
    startMetronomeFallback();
    setTimeout(()=>{ stopMetronomeFallback(); connectMetronomeEvents(); },2500);
  };
}
document.querySelectorAll("[data-metronome-toggle]").forEach(button=>button.addEventListener("click",()=>updateMetronome({running: metronomeState && metronomeState.running ? 0 : 1})));
document.querySelectorAll("[data-metronome-bpm-step]").forEach(button=>button.addEventListener("click",()=>{
  const current = Number(metronomeState?.bpm || document.querySelector(".metronome-bpm")?.value || 80);
  const next = Math.max(30, Math.min(240, current + Number(button.dataset.metronomeBpmStep || 0)));
  updateMetronome({bpm:next});
}));
document.querySelectorAll(".metronome-bpm").forEach(input=>input.addEventListener("change",()=>updateMetronome({bpm:Math.max(30,Math.min(240,Number(input.value || 80)))})));
document.querySelectorAll(".metronome-beats").forEach(select=>select.addEventListener("change",()=>updateMetronome({beatsPerBar:select.value})));
document.querySelectorAll(".metronome-subdivision").forEach(select=>select.addEventListener("change",()=>updateMetronome({subdivision:select.value})));
document.querySelectorAll(".metronome-volume").forEach(input=>input.addEventListener("input",()=>updateMetronome({volume:input.value})));
document.querySelectorAll(".metronome-output").forEach(select=>select.addEventListener("change",()=>updateMetronome({outputDevice:select.value})));
fetchMetronome();
connectMetronomeEvents();
let recorderState = null;
let recorderBusy = false;
function formatRecorderDuration(milliseconds){
  const totalSeconds = Math.max(0, Math.floor(Number(milliseconds || 0) / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor(totalSeconds % 3600 / 60);
  const seconds = totalSeconds % 60;
  return hours ? `${hours}:${String(minutes).padStart(2,"0")}:${String(seconds).padStart(2,"0")}` : `${minutes}:${String(seconds).padStart(2,"0")}`;
}
function formatFileSize(bytes){
  const value=Number(bytes || 0);
  return value < 1024 ? `${value} B` : value < 1024*1024 ? `${(value/1024).toFixed(1)} KB` : `${(value/1024/1024).toFixed(1)} MB`;
}
function renderRecorder(recorder){
  recorderState = recorder;
  if(recorder.error){
    $("recorderMessage").textContent=recorder.error;
    $("recorderMessage").classList.remove("ok");
    return;
  }
  $("recorderSummary").textContent = recorder.recording
    ? "正在标记 Take；60 分钟滚动缓存仍在运行"
    : `随时可保存最近的演奏 · 最多 ${recorder.bufferLimitMinutes || 60} 分钟`;
  $("recorderDuration").textContent=formatRecorderDuration(recorder.bufferedMs);
  $("recorderEvents").textContent=Number(recorder.eventCount || 0).toLocaleString();
  $("recorderTakeDuration").textContent=recorder.takeAvailable ? formatRecorderDuration(recorder.takeDurationMs) : "--";
  document.querySelectorAll("[data-recorder-action]").forEach(button=>{
    const action=button.dataset.recorderAction;
    button.disabled = recorderBusy
      || (action==="start" && recorder.recording)
      || (action==="stop" && !recorder.recording)
      || (action==="save-take" && !recorder.takeAvailable);
  });
  const recordings=recorder.recordings || [];
  $("recorderFileList").innerHTML=recordings.length ? recordings.map(file=>`
    <div class="recorder-file">
      <span title="${escapeAttr(file.name)}">${escapeHtml(file.name)}</span>
      <small>${new Date(Number(file.modifiedMs)).toLocaleString()} · ${formatFileSize(file.size)}</small>
      <a href="/recordings/download?name=${encodeURIComponent(file.name)}" download>下载</a>
    </div>`).join("") : "<span>还没有保存文件</span>";
}
async function fetchRecorder(){
  try{ renderRecorder(await fetch("/recorder",{cache:"no-store"}).then(response=>response.json())); }
  catch(e){
    $("recorderMessage").textContent="Recorder offline";
    $("recorderMessage").classList.remove("ok");
  }
}
async function recorderAction(button){
  if(recorderBusy) return;
  recorderBusy=true;
  renderRecorder(recorderState || {});
  $("recorderMessage").textContent="处理中…";
  $("recorderMessage").classList.remove("ok");
  const params=new URLSearchParams({action:button.dataset.recorderAction});
  if(button.dataset.seconds) params.set("seconds",button.dataset.seconds);
  try{
    const result=await fetch(`/recorder?${params.toString()}`,{cache:"no-store"}).then(response=>response.json());
    if(result.error) throw new Error(result.error);
    renderRecorder(result);
    $("recorderMessage").textContent=button.dataset.recorderAction.startsWith("save") ? "MIDI 已保存，可以在下方下载" : "";
    $("recorderMessage").classList.toggle("ok",!!$("recorderMessage").textContent);
  } catch(error){
    $("recorderMessage").textContent=error.message || "Recorder 操作失败";
    $("recorderMessage").classList.remove("ok");
  } finally {
    recorderBusy=false;
    if(recorderState) renderRecorder(recorderState);
  }
}
document.querySelectorAll("[data-recorder-action]").forEach(button=>button.addEventListener("click",()=>recorderAction(button)));
fetchRecorder();
setInterval(fetchRecorder,1000);
const clipState={
  sessionId:null, availableStartUs:0, availableEndUs:0, viewStartUs:0, viewEndUs:0,
  startUs:null, endUs:null, focus:"start", preset:"all", timeline:null, overview:null,
  previewId:null, preview:null, drag:null, overviewDrag:null, navigation:null,
  renderedStartUs:0, renderedEndUs:0, follow:false, loading:false, loadTimer:null
};
const clipMinimumUs=100000;
const clipMinimumViewUs=1000000;
const clipOverviewBins=240;
const clipDetailBins=600;
const maximumU64="18446744073709551615";
async function recorderApi(url, options={}){
  const response=await fetch(url,{cache:"no-store",...options});
  let data={};
  try{ data=await response.json(); }catch(e){}
  if(!response.ok || data.ok===false){
    const error=new Error(data.error?.message || `HTTP ${response.status}`);
    error.code=data.error?.code;
    error.details=data.error?.details || {};
    throw error;
  }
  return data;
}
function formatClipTime(us){
  const milliseconds=Math.max(0,Math.round(Number(us || 0)/1000));
  const minutes=Math.floor(milliseconds/60000);
  const seconds=Math.floor(milliseconds%60000/1000);
  const millis=milliseconds%1000;
  return `${minutes}:${String(seconds).padStart(2,"0")}.${String(millis).padStart(3,"0")}`;
}
function setClipStatus(message,className=""){
  $("clipStatus").textContent=message;
  $("clipStatus").className=`clip-status ${className}`.trim();
}
function clipPercent(us){
  const duration=Math.max(1,clipState.viewEndUs-clipState.viewStartUs);
  return Math.max(0,Math.min(100,(us-clipState.viewStartUs)/duration*100));
}
function overviewPercent(us){
  const duration=Math.max(1,clipState.availableEndUs-clipState.availableStartUs);
  return Math.max(0,Math.min(100,(us-clipState.availableStartUs)/duration*100));
}
function constrainClipView(start,end){
  const availableStart=clipState.availableStartUs;
  const availableEnd=clipState.availableEndUs;
  const total=Math.max(0,availableEnd-availableStart);
  if(total<=0) return [availableStart,availableEnd];
  const minimum=Math.min(clipMinimumViewUs,total);
  let duration=Math.max(minimum,Math.min(total,end-start));
  start=Math.max(availableStart,Math.min(start,availableEnd-duration));
  end=start+duration;
  if(end>availableEnd){
    end=availableEnd;
    start=end-duration;
  }
  return [Math.round(start),Math.round(end)];
}
function setClipView(start,end,{follow=false,preset=null,preview=true}={}){
  [clipState.viewStartUs,clipState.viewEndUs]=constrainClipView(start,end);
  clipState.follow=follow;
  clipState.preset=preset;
  renderClipNavigation();
  renderClipSelection();
  if(preview) previewClipBinsForView();
}
function renderClipNavigation(){
  const total=clipState.availableEndUs-clipState.availableStartUs;
  const empty=total<=0;
  $("clipOverview").classList.toggle("empty",empty);
  $("clipRail").classList.toggle("empty",empty);
  document.querySelectorAll("[data-clip-view],[data-clip-navigation]").forEach(button=>button.disabled=empty);
  if(empty){
    $("clipViewStart").textContent="--";
    $("clipViewEnd").textContent="--";
    $("clipFollowStatus").textContent="等待 MIDI";
    return;
  }
  const viewStart=overviewPercent(clipState.viewStartUs);
  const viewEnd=overviewPercent(clipState.viewEndUs);
  const selectionStart=overviewPercent(clipState.startUs ?? clipState.viewStartUs);
  const selectionEnd=overviewPercent(clipState.endUs ?? clipState.viewEndUs);
  $("clipOverviewViewport").style.left=`${viewStart}%`;
  $("clipOverviewViewport").style.width=`${Math.max(0,viewEnd-viewStart)}%`;
  $("clipOverviewSelection").style.left=`${selectionStart}%`;
  $("clipOverviewSelection").style.width=`${Math.max(0,selectionEnd-selectionStart)}%`;
  $("clipViewStart").textContent=formatClipTime(clipState.viewStartUs);
  $("clipViewEnd").textContent=formatClipTime(clipState.viewEndUs);
  $("clipFollowStatus").textContent=clipState.follow ? "跟随最新" : (clipState.preset==="all" ? "全部缓存" : "浏览缓存");
  const latestButton=document.querySelector('[data-clip-navigation="latest"]');
  latestButton.classList.toggle("following",clipState.follow);
  latestButton.textContent=clipState.follow ? "正在跟随" : "回到最新";
  document.querySelectorAll("[data-clip-view]").forEach(button=>button.classList.toggle("active",button.dataset.clipView===String(clipState.preset)));
}
function previewClipBinsForView(){
  if(!clipState.renderedEndUs || !$("clipRail").getBoundingClientRect().width) return;
  const oldDuration=Math.max(1,clipState.renderedEndUs-clipState.renderedStartUs);
  const newDuration=Math.max(1,clipState.viewEndUs-clipState.viewStartUs);
  const scale=oldDuration/newDuration;
  const offset=(clipState.renderedStartUs-clipState.viewStartUs)/newDuration*$("clipRail").getBoundingClientRect().width;
  $("clipBins").style.transform=`translateX(${offset}px) scaleX(${scale})`;
  $("clipBins").style.opacity=".68";
}
function scheduleClipTimelineLoad(delay=100){
  clearTimeout(clipState.loadTimer);
  clipState.loadTimer=setTimeout(()=>loadClipTimeline(),delay);
}
function renderClipSelection(){
  if(clipState.startUs==null || clipState.endUs==null) return;
  const hasRange=clipState.endUs-clipState.startUs>=clipMinimumUs;
  const start=clipPercent(clipState.startUs);
  const end=clipPercent(clipState.endUs);
  $("clipOutsideStart").style.width=`${start}%`;
  $("clipOutsideEnd").style.width=`${100-end}%`;
  $("clipSelection").style.left=`${start}%`;
  $("clipSelection").style.width=`${Math.max(0,end-start)}%`;
  $("clipStartHandle").style.left=`clamp(22px, ${start}%, calc(100% - 22px))`;
  $("clipEndHandle").style.left=`clamp(22px, ${end}%, calc(100% - 22px))`;
  $("clipStartTime").textContent=formatClipTime(clipState.startUs);
  $("clipEndTime").textContent=formatClipTime(clipState.endUs);
  $("clipDuration").textContent=formatClipTime(clipState.endUs-clipState.startUs);
  for(const [element,value,label] of [
    [$("clipStartHandle"),clipState.startUs,"片段开始"],
    [$("clipEndHandle"),clipState.endUs,"片段结束"]
  ]){
    element.setAttribute("aria-valuemin",String(clipState.availableStartUs));
    element.setAttribute("aria-valuemax",String(clipState.availableEndUs));
    element.setAttribute("aria-valuenow",String(value));
    element.setAttribute("aria-valuetext",`${label} ${formatClipTime(value)}`);
  }
  document.querySelectorAll("[data-clip-focus]").forEach(button=>button.classList.toggle("active",button.dataset.clipFocus===clipState.focus));
  $("clipPlay").disabled=!hasRange;
  $("clipSave").disabled=!hasRange;
  document.querySelectorAll("[data-clip-nudge]").forEach(button=>button.disabled=!hasRange);
  renderClipNavigation();
}
function renderClipBins(timeline){
  clipState.timeline=timeline;
  const maximum=Math.max(1,...timeline.bins.map(bin=>Number(bin.noteOnCount||0)));
  $("clipBins").innerHTML=timeline.bins.map(bin=>{
    const density=Number(bin.noteOnCount||0)/maximum;
    const velocity=Number(bin.velocityMaximum||0)/127;
    const height=Math.max(3,Math.round((density*.72+velocity*.28)*92));
    const opacity=.38+Math.min(.58,density*.42+velocity*.22);
    return `<span class="clip-bin" style="height:${height}%;opacity:${opacity}"></span>`;
  }).join("");
  clipState.renderedStartUs=timeline.range.startUs;
  clipState.renderedEndUs=timeline.range.endUs;
  $("clipBins").style.transform="";
  $("clipBins").style.opacity="";
  renderClipSelection();
}
function renderClipOverview(timeline){
  clipState.overview=timeline;
  const maximum=Math.max(1,...timeline.bins.map(bin=>Number(bin.noteOnCount||0)));
  $("clipOverviewBins").innerHTML=timeline.bins.map(bin=>{
    const density=Number(bin.noteOnCount||0)/maximum;
    const velocity=Number(bin.velocityMaximum||0)/127;
    const height=Math.max(4,Math.round((density*.72+velocity*.28)*92));
    const opacity=.34+Math.min(.62,density*.44+velocity*.24);
    return `<span class="clip-overview-bin" style="height:${height}%;opacity:${opacity}"></span>`;
  }).join("");
  renderClipNavigation();
}
function selectedClipParams(){
  return new URLSearchParams({
    sessionId:clipState.sessionId,
    startUs:String(Math.round(clipState.startUs)),
    endUs:String(Math.round(clipState.endUs))
  });
}
async function loadClipTimeline({reset=false,message=""}={}){
  if(clipState.loading) return;
  clipState.loading=true;
  try{
    const overviewParams=new URLSearchParams({
      startUs:"0",endUs:maximumU64,bins:String(clipOverviewBins),notes:"0"
    });
    const overview=await recorderApi(`/recorder/timeline?${overviewParams}`);
    const changed=Boolean(clipState.sessionId && clipState.sessionId!==overview.sessionId);
    clipState.sessionId=overview.sessionId;
    clipState.availableStartUs=overview.buffer.availableStartUs;
    clipState.availableEndUs=overview.buffer.availableEndUs;
    const previousViewDuration=Math.max(clipMinimumViewUs,clipState.viewEndUs-clipState.viewStartUs);
    if(reset || changed || clipState.startUs==null){
      clipState.follow=false;
      clipState.preset="all";
      clipState.viewStartUs=clipState.availableStartUs;
      clipState.viewEndUs=clipState.availableEndUs;
      const take=overview.take && !overview.take.recording ? overview.take : null;
      clipState.startUs=take?.startUs ?? clipState.viewStartUs;
      clipState.endUs=take?.endUs ?? clipState.viewEndUs;
    }else if(clipState.follow){
      clipState.viewEndUs=clipState.availableEndUs;
      clipState.viewStartUs=Math.max(clipState.availableStartUs,clipState.viewEndUs-previousViewDuration);
    }else{
      [clipState.viewStartUs,clipState.viewEndUs]=constrainClipView(clipState.viewStartUs,clipState.viewEndUs);
    }
    if(clipState.availableEndUs<=clipState.availableStartUs){
      clipState.startUs=clipState.availableStartUs;
      clipState.endUs=clipState.availableEndUs;
      clipState.viewStartUs=clipState.availableStartUs;
      clipState.viewEndUs=clipState.availableEndUs;
      renderClipOverview(overview);
      renderClipBins(overview);
      $("clipEventCount").textContent="0";
      $("clipNoteCount").textContent="0";
      setClipStatus("等待 MIDI 输入后即可选择片段。");
      return;
    }
    let clipped=false;
    if(clipState.startUs<clipState.availableStartUs){
      clipState.startUs=clipState.availableStartUs;
      clipped=true;
    }
    clipState.endUs=Math.min(clipState.endUs,clipState.availableEndUs);
    if(clipState.endUs-clipState.startUs<clipMinimumUs){
      clipState.endUs=Math.min(clipState.availableEndUs,clipState.startUs+clipMinimumUs);
    }
    const detailParams=new URLSearchParams({
      sessionId:clipState.sessionId,
      startUs:String(Math.round(clipState.viewStartUs)),
      endUs:String(Math.round(clipState.viewEndUs)),
      bins:String(clipDetailBins),
      notes:"1"
    });
    const timeline=await recorderApi(`/recorder/timeline?${detailParams}`);
    clipState.viewStartUs=timeline.range.startUs;
    clipState.viewEndUs=timeline.range.endUs;
    renderClipOverview(overview);
    renderClipBins(timeline);
    await refreshClipStats();
    if(changed) setClipStatus("Monitor 已重启，旧选择已清除。","error");
    else if(clipped) setClipStatus("最旧的片段已离开滚动缓存，开始位置已调整。","error");
    else if(message) setClipStatus(message);
    else if(!timeline.stateComplete) setClipStatus("缓存左边界之前的 MIDI 状态不完整；保存和试听会标记警告。");
    else setClipStatus("拖动手柄或所选区域来精确选择片段。");
  }catch(error){
    if(error.code==="stale_session"){
      clipState.sessionId=null;
      clipState.startUs=null;
      clipState.endUs=null;
      setClipStatus("Monitor 已重启，正在重新载入。","error");
      setTimeout(()=>loadClipTimeline({reset:true}),250);
    }else setClipStatus(error.message || "无法载入时间轴","error");
  }finally{ clipState.loading=false; }
}
async function refreshClipStats(){
  if(!clipState.sessionId || clipState.startUs==null) return;
  try{
    const params=selectedClipParams();
    params.set("bins","50");
    const data=await recorderApi(`/recorder/timeline?${params}`);
    $("clipEventCount").textContent=data.bins.reduce((sum,bin)=>sum+Number(bin.eventCount||0),0).toLocaleString();
    $("clipNoteCount").textContent=data.bins.reduce((sum,bin)=>sum+Number(bin.noteOnCount||0),0).toLocaleString();
  }catch(error){
    $("clipEventCount").textContent="--";
    $("clipNoteCount").textContent="--";
  }
}
async function stopClipPreview(silent=false){
  if(!clipState.previewId) return;
  const previewId=clipState.previewId;
  try{
    await recorderApi(`/recorder/preview?action=stop&previewId=${previewId}`,{
      method:"POST",headers:{"X-FP10-Control":"1"}
    });
    if(!silent) setClipStatus("试听已停止。");
  }catch(error){
    if(error.code!=="preview_replaced" && !silent) setClipStatus(error.message,"error");
  }
  clipState.previewId=null;
  $("clipPlayhead").style.display="none";
}
function constrainClipSelection(start,end){
  const availableStart=Math.max(clipState.availableStartUs,clipState.viewStartUs);
  const availableEnd=Math.min(clipState.availableEndUs,clipState.viewEndUs);
  start=Math.max(availableStart,Math.min(start,availableEnd-clipMinimumUs));
  end=Math.min(availableEnd,Math.max(end,start+clipMinimumUs));
  return [Math.round(start),Math.round(end)];
}
function beginClipDrag(event,type){
  if(clipState.startUs==null) return;
  stopClipPreview(true);
  clipState.follow=false;
  clipState.preset=null;
  event.currentTarget.setPointerCapture(event.pointerId);
  clipState.drag={type,pointerId:event.pointerId,x:event.clientX,startUs:clipState.startUs,endUs:clipState.endUs};
  if(type==="start" || type==="end") clipState.focus=type;
  renderClipSelection();
  if(event.pointerType!=="touch") event.stopPropagation();
  event.preventDefault();
}
function moveClipDrag(event){
  const drag=clipState.drag;
  if(!drag || drag.pointerId!==event.pointerId) return;
  const width=Math.max(1,$("clipRail").getBoundingClientRect().width);
  const delta=(event.clientX-drag.x)/width*(clipState.viewEndUs-clipState.viewStartUs);
  let start=drag.startUs,end=drag.endUs;
  if(drag.type==="start") start=Math.min(end-clipMinimumUs,start+delta);
  else if(drag.type==="end") end=Math.max(start+clipMinimumUs,end+delta);
  else {
    const duration=end-start;
    start+=delta; end+=delta;
    if(start<clipState.availableStartUs){ start=clipState.availableStartUs; end=start+duration; }
    if(end>clipState.availableEndUs){ end=clipState.availableEndUs; start=end-duration; }
    clipState.startUs=Math.round(start);
    clipState.endUs=Math.round(end);
    renderClipSelection();
    return;
  }
  [clipState.startUs,clipState.endUs]=constrainClipSelection(start,end);
  renderClipSelection();
}
function endClipDrag(event){
  if(!clipState.drag || clipState.drag.pointerId!==event.pointerId) return;
  clipState.drag=null;
  refreshClipStats();
}
$("clipStartHandle").addEventListener("pointerdown",event=>beginClipDrag(event,"start"));
$("clipEndHandle").addEventListener("pointerdown",event=>beginClipDrag(event,"end"));
$("clipSelection").addEventListener("pointerdown",event=>beginClipDrag(event,"region"));
[$("clipStartHandle"),$("clipEndHandle"),$("clipSelection")].forEach(element=>{
  element.addEventListener("pointermove",moveClipDrag);
  element.addEventListener("pointerup",endClipDrag);
  element.addEventListener("pointercancel",endClipDrag);
});
function beginClipNavigation(event){
  if(clipState.availableEndUs<=clipState.availableStartUs) return;
  const editingTarget=Boolean(event.target.closest(".clip-handle,.clip-selection"));
  if(!editingTarget) $("clipRail").setPointerCapture(event.pointerId);
  if(!clipState.navigation){
    clipState.navigation={pointers:new Map(),mode:"pan"};
  }
  const navigation=clipState.navigation;
  navigation.pointers.set(event.pointerId,{x:event.clientX,y:event.clientY});
  clipState.follow=false;
  clipState.preset=null;
  if(navigation.pointers.size===1){
    navigation.mode=editingTarget ? "edit" : "pan";
    navigation.startX=event.clientX;
    navigation.startViewStartUs=clipState.viewStartUs;
    navigation.startViewEndUs=clipState.viewEndUs;
  }else if(navigation.pointers.size===2){
    clipState.drag=null;
    const points=[...navigation.pointers.values()];
    const rect=$("clipRail").getBoundingClientRect();
    navigation.mode="pinch";
    navigation.startDistance=Math.max(1,Math.hypot(points[1].x-points[0].x,points[1].y-points[0].y));
    navigation.startDuration=clipState.viewEndUs-clipState.viewStartUs;
    navigation.anchorUs=clipState.viewStartUs+(((points[0].x+points[1].x)/2-rect.left)/Math.max(1,rect.width))*navigation.startDuration;
  }
  $("clipRail").classList.add("navigating");
  event.preventDefault();
}
function moveClipNavigation(event){
  const navigation=clipState.navigation;
  if(!navigation || !navigation.pointers.has(event.pointerId)) return;
  navigation.pointers.set(event.pointerId,{x:event.clientX,y:event.clientY});
  const rect=$("clipRail").getBoundingClientRect();
  const width=Math.max(1,rect.width);
  if(navigation.mode==="pinch" && navigation.pointers.size>=2){
    const points=[...navigation.pointers.values()].slice(0,2);
    const distance=Math.max(1,Math.hypot(points[1].x-points[0].x,points[1].y-points[0].y));
    const centerX=(points[0].x+points[1].x)/2;
    const total=clipState.availableEndUs-clipState.availableStartUs;
    const duration=Math.max(Math.min(clipMinimumViewUs,total),Math.min(total,navigation.startDuration*navigation.startDistance/distance));
    const fraction=Math.max(0,Math.min(1,(centerX-rect.left)/width));
    const start=navigation.anchorUs-fraction*duration;
    setClipView(start,start+duration,{preview:true});
  }else if(navigation.mode==="pan" && navigation.pointers.size===1){
    const duration=navigation.startViewEndUs-navigation.startViewStartUs;
    const delta=-(event.clientX-navigation.startX)/width*duration;
    setClipView(navigation.startViewStartUs+delta,navigation.startViewEndUs+delta,{preview:true});
  }
  event.preventDefault();
}
function endClipNavigation(event){
  const navigation=clipState.navigation;
  if(!navigation || !navigation.pointers.has(event.pointerId)) return;
  navigation.pointers.delete(event.pointerId);
  if(navigation.pointers.size===1){
    const remaining=[...navigation.pointers.values()][0];
    navigation.mode="pan";
    navigation.startX=remaining.x;
    navigation.startViewStartUs=clipState.viewStartUs;
    navigation.startViewEndUs=clipState.viewEndUs;
  }else if(navigation.pointers.size===0){
    clipState.navigation=null;
    $("clipRail").classList.remove("navigating");
    scheduleClipTimelineLoad(0);
  }
}
$("clipRail").addEventListener("pointerdown",beginClipNavigation);
$("clipRail").addEventListener("pointermove",moveClipNavigation);
$("clipRail").addEventListener("pointerup",endClipNavigation);
$("clipRail").addEventListener("pointercancel",endClipNavigation);
function zoomClipView(factor,anchorUs=(clipState.viewStartUs+clipState.viewEndUs)/2){
  const oldDuration=clipState.viewEndUs-clipState.viewStartUs;
  const newDuration=oldDuration*factor;
  const fraction=oldDuration>0 ? (anchorUs-clipState.viewStartUs)/oldDuration : .5;
  const start=anchorUs-fraction*newDuration;
  setClipView(start,start+newDuration,{preview:true});
}
$("clipRail").addEventListener("wheel",event=>{
  if(!(event.ctrlKey || event.metaKey)) return;
  const rect=$("clipRail").getBoundingClientRect();
  const fraction=Math.max(0,Math.min(1,(event.clientX-rect.left)/Math.max(1,rect.width)));
  const anchor=clipState.viewStartUs+fraction*(clipState.viewEndUs-clipState.viewStartUs);
  clipState.follow=false;
  clipState.preset=null;
  zoomClipView(Math.exp(event.deltaY*.002),anchor);
  scheduleClipTimelineLoad();
  event.preventDefault();
},{passive:false});
$("clipRail").addEventListener("keydown",event=>{
  if(!["+", "=", "-", "_", "Home"].includes(event.key)) return;
  if(event.key==="Home"){
    clipState.follow=true;
    clipState.preset=null;
    const duration=Math.min(120000000,clipState.availableEndUs-clipState.availableStartUs);
    setClipView(clipState.availableEndUs-duration,clipState.availableEndUs,{follow:true,preview:true});
  }else{
    clipState.follow=false;
    clipState.preset=null;
    zoomClipView(["+","="].includes(event.key) ? .5 : 2);
  }
  scheduleClipTimelineLoad(0);
  event.preventDefault();
});
function beginOverviewDrag(event,type){
  if(clipState.availableEndUs<=clipState.availableStartUs) return;
  stopClipPreview(true);
  event.currentTarget.setPointerCapture(event.pointerId);
  clipState.follow=false;
  clipState.preset=null;
  clipState.overviewDrag={
    type,pointerId:event.pointerId,x:event.clientX,
    startUs:clipState.viewStartUs,endUs:clipState.viewEndUs
  };
  event.stopPropagation();
  event.preventDefault();
}
function moveOverviewDrag(event){
  const drag=clipState.overviewDrag;
  if(!drag || drag.pointerId!==event.pointerId) return;
  const width=Math.max(1,$("clipOverview").getBoundingClientRect().width);
  const total=clipState.availableEndUs-clipState.availableStartUs;
  const delta=(event.clientX-drag.x)/width*total;
  let start=drag.startUs,end=drag.endUs;
  if(drag.type==="start") start=Math.min(end-clipMinimumViewUs,start+delta);
  else if(drag.type==="end") end=Math.max(start+clipMinimumViewUs,end+delta);
  else { start+=delta; end+=delta; }
  setClipView(start,end,{preview:true});
  event.preventDefault();
}
function endOverviewDrag(event){
  if(!clipState.overviewDrag || clipState.overviewDrag.pointerId!==event.pointerId) return;
  clipState.overviewDrag=null;
  scheduleClipTimelineLoad(0);
}
$("clipOverviewViewport").addEventListener("pointerdown",event=>beginOverviewDrag(event,"move"));
$("clipOverviewStartHandle").addEventListener("pointerdown",event=>beginOverviewDrag(event,"start"));
$("clipOverviewEndHandle").addEventListener("pointerdown",event=>beginOverviewDrag(event,"end"));
[$("clipOverviewViewport"),$("clipOverviewStartHandle"),$("clipOverviewEndHandle")].forEach(element=>{
  element.addEventListener("pointermove",moveOverviewDrag);
  element.addEventListener("pointerup",endOverviewDrag);
  element.addEventListener("pointercancel",endOverviewDrag);
});
$("clipOverview").addEventListener("pointerdown",event=>{
  if(event.target!==$("clipOverview") && event.target!==$("clipOverviewBins")) return;
  const rect=$("clipOverview").getBoundingClientRect();
  const fraction=Math.max(0,Math.min(1,(event.clientX-rect.left)/Math.max(1,rect.width)));
  const center=clipState.availableStartUs+fraction*(clipState.availableEndUs-clipState.availableStartUs);
  const duration=clipState.viewEndUs-clipState.viewStartUs;
  clipState.follow=false;
  clipState.preset=null;
  setClipView(center-duration/2,center+duration/2,{preview:true});
  scheduleClipTimelineLoad(0);
  event.preventDefault();
});
function nudgeClipBoundary(delta){
  if(clipState.startUs==null) return;
  stopClipPreview(true);
  let start=clipState.startUs,end=clipState.endUs;
  if(clipState.focus==="start") start+=delta; else end+=delta;
  [clipState.startUs,clipState.endUs]=constrainClipSelection(start,end);
  renderClipSelection();
  refreshClipStats();
}
[$("clipStartHandle"),$("clipEndHandle")].forEach((handle,index)=>handle.addEventListener("keydown",event=>{
  if(!["ArrowLeft","ArrowRight"].includes(event.key)) return;
  clipState.focus=index===0?"start":"end";
  nudgeClipBoundary((event.key==="ArrowLeft"?-1:1)*(event.shiftKey?1000000:100000));
  event.preventDefault();
}));
document.querySelectorAll("[data-clip-focus]").forEach(button=>button.addEventListener("click",()=>{
  clipState.focus=button.dataset.clipFocus;
  renderClipSelection();
  $(clipState.focus==="start"?"clipStartHandle":"clipEndHandle").focus();
}));
document.querySelectorAll("[data-clip-nudge]").forEach(button=>button.addEventListener("click",()=>nudgeClipBoundary(Number(button.dataset.clipNudge))));
document.querySelectorAll("[data-clip-view]").forEach(button=>button.addEventListener("click",async()=>{
  if(!clipState.sessionId) return;
  stopClipPreview(true);
  const value=button.dataset.clipView;
  const duration=value==="all" ? clipState.availableEndUs-clipState.availableStartUs : Number(value);
  clipState.preset=value;
  clipState.follow=false;
  setClipView(clipState.availableEndUs-duration,clipState.availableEndUs,{preset:value,preview:true});
  await loadClipTimeline({message:"时间轴范围已更新。"});
}));
document.querySelectorAll("[data-clip-navigation]").forEach(button=>button.addEventListener("click",async()=>{
  if(!clipState.sessionId) return;
  stopClipPreview(true);
  const action=button.dataset.clipNavigation;
  if(action==="latest"){
    const total=clipState.availableEndUs-clipState.availableStartUs;
    const current=clipState.viewEndUs-clipState.viewStartUs;
    const duration=Math.min(total,clipState.preset==="all" ? 120000000 : Math.max(clipMinimumViewUs,current));
    setClipView(clipState.availableEndUs-duration,clipState.availableEndUs,{follow:true,preview:true});
  }else if(action==="fit-selection"){
    const duration=Math.max(clipMinimumViewUs,clipState.endUs-clipState.startUs);
    const padding=duration*.08;
    setClipView(clipState.startUs-padding,clipState.endUs+padding,{preview:true});
  }else{
    zoomClipView(action==="zoom-in" ? .5 : 2);
  }
  await loadClipTimeline({message:action==="latest" ? "正在跟随最新 MIDI。" : "时间轴范围已更新。"});
}));
$("clipPlay").addEventListener("click",async()=>{
  if(!clipState.sessionId) return;
  await stopClipPreview(true);
  setClipStatus("正在连接电脑的 MIDI 输出…");
  const params=selectedClipParams();
  params.set("action","play");
  params.set("outputName",$("clipOutput").value);
  try{
    const data=await recorderApi(`/recorder/preview?${params}`,{
      method:"POST",headers:{"X-FP10-Control":"1"}
    });
    clipState.previewId=data.previewId;
    setClipStatus("试听中；滚动录音捕获已暂停。","ok");
  }catch(error){ setClipStatus(error.message || "试听失败","error"); }
});
$("clipStop").addEventListener("click",()=>stopClipPreview());
$("clipSave").addEventListener("click",async()=>{
  if(!clipState.sessionId) return;
  await stopClipPreview(true);
  setClipStatus("正在保存精确片段…");
  try{
    const result=await recorderApi(`/recorder/save-range?${selectedClipParams()}`,{
      method:"POST",headers:{"X-FP10-Control":"1"}
    });
    const warnings=result.warnings?.length ? `（${result.warnings.join("、")}）` : "";
    setClipStatus(`已保存 ${result.recording.name}${warnings}`,"ok");
    fetchRecorder();
  }catch(error){
    if(error.code==="range_expired") loadClipTimeline();
    setClipStatus(error.message || "保存失败","error");
  }
});
async function pollClipPreview(){
  if(!$("clipEditor").open) return;
  try{
    const data=await recorderApi("/recorder/preview");
    const preview=data.preview;
    clipState.preview=preview;
    if(data.outputs?.length){
      const selected=$("clipOutput").value;
      $("clipOutput").innerHTML=data.outputs.map(name=>`<option value="${escapeAttr(name)}">${escapeHtml(name)}</option>`).join("");
      $("clipOutput").value=data.outputs.includes(selected) ? selected : (data.outputs.find(name=>name.toLowerCase().includes("fp10 mapped")) || data.outputs[0]);
    }
    if(clipState.previewId!=null && preview.previewId===clipState.previewId && preview.state==="playing" && preview.positionUs!=null){
      $("clipPlayhead").style.display="block";
      $("clipPlayhead").style.left=`${clipPercent(preview.positionUs)}%`;
      setClipStatus("试听中；滚动录音捕获已暂停。","ok");
    }else if(clipState.previewId!=null && preview.previewId===clipState.previewId && preview.state!=="playing"){
      clipState.previewId=null;
      $("clipPlayhead").style.display="none";
      if(preview.error) setClipStatus(preview.error,"error");
      else setClipStatus("试听完成，滚动录音捕获已恢复。","ok");
    }
  }catch(error){}
}
$("clipEditor").addEventListener("toggle",()=>{
  if($("clipEditor").open) loadClipTimeline({reset:!clipState.sessionId});
  else stopClipPreview(true);
});
setInterval(()=>{ if($("clipEditor").open) loadClipTimeline(); },2000);
setInterval(pollClipPreview,500);
restoreSettings();
$("keySelect").addEventListener("change",()=>{ updateSettings({key:$("keySelect").value}); if(state.latestData) render(state.latestData); });
let pollingTimer = null;
let eventSource = null;
async function fetchState(){ try{ const data=await fetch("/state",{cache:"no-store"}).then(r=>r.json()); render(data); } catch(e){ $("status").textContent="backend offline"; } }
async function fetchBuildVersion(){
  try{
    const data=await fetch("/version",{cache:"no-store"}).then(response=>response.json());
    $("buildVersion").textContent=`FP10 Monitor v${data.version} · ${data.build}`;
  }catch(error){
    $("buildVersion").textContent="FP10 Monitor · 版本未知";
  }
}
function startPolling(){ if(pollingTimer) return; pollingTimer=setInterval(fetchState,220); fetchState(); }
function connectEvents(){
  if(!window.EventSource){ startPolling(); return; }
  eventSource = new EventSource("/events");
  eventSource.addEventListener("state",(event)=>{ try{ render(JSON.parse(event.data)); } catch(e){} });
  eventSource.onerror = ()=>{ $("status").textContent="reconnecting"; if(eventSource){ eventSource.close(); eventSource=null; } startPolling(); setTimeout(()=>{ if(pollingTimer){ clearInterval(pollingTimer); pollingTimer=null; } connectEvents(); },2500); };
}
fetchBuildVersion();
connectEvents();
