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
restoreSettings();
$("keySelect").addEventListener("change",()=>{ updateSettings({key:$("keySelect").value}); if(state.latestData) render(state.latestData); });
let pollingTimer = null;
let eventSource = null;
async function fetchState(){ try{ const data=await fetch("/state",{cache:"no-store"}).then(r=>r.json()); render(data); } catch(e){ $("status").textContent="backend offline"; } }
function startPolling(){ if(pollingTimer) return; pollingTimer=setInterval(fetchState,220); fetchState(); }
function connectEvents(){
  if(!window.EventSource){ startPolling(); return; }
  eventSource = new EventSource("/events");
  eventSource.addEventListener("state",(event)=>{ try{ render(JSON.parse(event.data)); } catch(e){} });
  eventSource.onerror = ()=>{ $("status").textContent="reconnecting"; if(eventSource){ eventSource.close(); eventSource=null; } startPolling(); setTimeout(()=>{ if(pollingTimer){ clearInterval(pollingTimer); pollingTimer=null; } connectEvents(); },2500); };
}
connectEvents();
