"use strict";
(() => {
  const A = window.AEGIS;
  if (!A) return;
  const { invoke, esc, log, S } = A;
  const $ = (id) => document.getElementById(id);
  const text = (en, ru, kk) => window.I18N.lang === "ru" ? ru : window.I18N.lang === "kk" ? kk : en;
  const t = (s) => window.I18N.t(s);
  const clone = (x) => JSON.parse(JSON.stringify(x));
  const B = { info: null, p: null, result: null, busy: false, saving: false, editingSetup: false, setupName: "", setupError: "", stale: false, progress: "", error: "", from: "2026-04-01", to: "2026-09-30" };
  let saveTimer;
  function save() {
    clearTimeout(saveTimer);
    const p = clone(B.p);
    saveTimer = setTimeout(() => invoke("structural_save", { params: p, name: null }).catch(e => log(String(e), "bad")), 400);
  }
  function changed() { B.stale = true; save(); const el = $("stStale"); if (el) el.hidden = false; }
  const n = (v, d = 3) => Number.isFinite(v) ? v.toFixed(d) : "—";
  const date = (s) => new Date(s * 1000).toISOString().slice(0, 19).replace("T", " ");
  function control(s) {
    const v = B.p.values[s.id], dis = B.busy ? "disabled" : "";
    const help = s.group === "Costs" ? text("Assumed costs, not measured account execution.", "Предполагаемые издержки, не измеренное исполнение счёта.", "Болжамды шығындар, нақты шот орындалуы емес.") : "";
    return `<div class="bt-row slider" title="${esc(help)}"><label class="bt-label" for="st-${s.id}">${esc(t(s.label))}</label>${s.toggle
      ? `<input id="st-${s.id}" type="checkbox" data-param="${s.id}" ${v ? "checked" : ""} ${dis}>`
      : `<input id="st-${s.id}" type="range" data-param="${s.id}" min="${s.lo}" max="${s.hi}" step="${s.step}" value="${v}" ${dis}><output>${n(v, String(s.step).includes(".") ? 2 : 0)}</output>`}</div>`;
  }
  function results() {
    const r = B.result;
    const error = B.error ? `<p class="hint warn" role="alert">${esc(B.error)}</p>` : "";
    const action = `<div class="bt-run"><button type="button" class="cta" id="stRun" ${B.busy ? "disabled" : ""}>${esc(t(B.busy ? "Running…" : "Run backtest"))}</button><span id="stProgress" class="hint" role="status">${esc(B.progress)}</span></div>`;
    if (!r) return action + error + `<p class="hint">${esc(text("Bybit XAUUSDT trade tape; archives download once and are cached. Six months can use hundreds of MB of RAM. No live orders.", "Лента сделок Bybit XAUUSDT: архивы скачиваются один раз и кэшируются. Шесть месяцев могут занять сотни МБ RAM. Реальные ордера не выставляются.", "Bybit XAUUSDT мәміле таспасы бір рет жүктеліп, кэшке сақталады. Нақты ордерлер жоқ."))}</p>`;
    const cards = [[t("Trades"), r.trades.length], [t("Total R"), n(r.total_r)], [t("Avg R"), n(r.avg_r)], [t("Win rate"), n(r.win_rate * 100, 1) + "%"], [t("Max drawdown"), n(r.max_dd_r) + " R"]];
    let eq = "";
    if (r.equity.length) {
      const v = [0, ...r.equity.map(p => p[1])], lo = Math.min(...v), hi = Math.max(...v), span = Math.max(hi-lo, .001);
      const points = r.equity.map((p,i) => `${i/(Math.max(1,r.equity.length-1))*550},${120-(p[1]-lo)/span*110}`).join(" ");
      eq = `<svg class="bt-equity" viewBox="0 0 560 130" aria-label="Equity R"><polyline points="${points}" fill="none" stroke="var(--pos)" stroke-width="2"/></svg>`;
    }
    const tr = r.trades.slice(-100).reverse().map(x => `<tr><td>${date(x.entry)}</td><td>${x.side > 0 ? t("Long") : t("Short")}</td><td>${n(x.entry_px,2)}</td><td>${n(x.stop,2)}</td><td>${n(x.target,2)}</td><td>${n((x.target-x.entry_px)/(x.entry_px-x.stop),2)}</td><td class="${x.net_r>0 ? "pos" : "neg"}">${n(x.net_r)}</td><td>${esc(t(x.reason))}</td></tr>`).join("");
    return action + error + `<p id="stStale" class="hint warn" ${B.stale ? "" : "hidden"}>${esc(text("Settings changed — rerun. These results belong to the previous settings.", "Настройки изменены — повторите тест. Результат относится к прежним настройкам.", "Баптаулар өзгерді — тестті қайталаңыз."))}</p>
      <div class="bt-cards">${cards.map(([k,v])=>`<div class="bt-card"><span>${esc(k)}</span><strong>${esc(String(v))}</strong></div>`).join("")}</div>${eq}
      <p class="hint">${esc(text("Signal funnel", "Воронка сигналов", "Сигнал сүзгісі"))}: ${r.signals}; ${esc(text("expired / target passed / costs / risk / overlap", "истекли / цель пройдена / издержки / риск / занято", "мерзім / мақсат / шығын / тәуекел / позиция"))}: ${r.expired} / ${r.target_passed} / ${r.cost_rejected} / ${r.risk_rejected} / ${r.overlapping}. ${esc(text("Censored", "Незавершённые", "Аяқталмаған"))}: ${r.censored}.</p>
      <div class="bt-table"><table><tr><th>UTC</th><th>${esc(t("Direction"))}</th><th>${esc(t("Entry"))}</th><th>${esc(t("Stop"))}</th><th>${esc(t("Target"))}</th><th>R/R</th><th>Net R</th><th>${esc(t("Exit"))}</th></tr>${tr}</table></div>
      <div class="bt-table"><table><tr><th>${esc(t("Month"))}</th><th>${esc(t("Trades"))}</th><th>Net R</th></tr>${Object.entries(r.months).map(([k,a])=>`<tr><td>${esc(k)}</td><td>${a[0]}</td><td>${n(a[1])}</td></tr>`).join("")}</table></div>
      <p class="hint">${esc(text("Tape-price fills are proxies. Quotes, FIFO, funding and liquidation are not fully modeled. R is not account return. The target stays mapped from the original level: remaining R/R may be small.", "Исполнения по ленте — модель. Котировки, FIFO, funding и ликвидация полностью не учтены. R не равен доходности счёта. Цель остаётся от исходного уровня: остаточное R/R может быть малым.", "Таспа орындалуы — модель. FIFO, funding толық есептелмеген. R шот кірісі емес."))}</p>`;
  }
  function render(body) {
    body = body || $("panelBody");
    if (!B.info) {
      body.innerHTML = `<p class="hint">${esc(t("Loading…"))}</p>`;
      invoke("structural_info").then(i => { B.info = i; B.p = clone(i.saved || i.defaults); if(S.panel === "structural") render(); }).catch(e=>{body.textContent=String(e);});return;
    }
    const groups = [...new Set(B.info.specs.map(s=>s.group))];
    const presets = Object.keys(B.info.presets || {}).map(k=>`<button type="button" class="ghost sm" data-preset="${esc(k)}">${esc(k)}</button>`).join("");
    body.innerHTML = `<div class="bt-cols"><div class="bt-col" id="stSettings">
      <div class="st-setups"><button type="button" class="st-research" id="stResearch">${esc(text("Setup 1 +", "Сетап 1 +", "Сетап 1 +"))} · +0.85R / 5 ${esc(t("Trades"))}</button>
      <button type="button" class="ghost sm" id="stAddSetup" ${B.busy || B.saving ? "disabled" : ""}>+ ${esc(text("Add my setup", "Добавить свой сетап", "Өз сетапымды қосу"))}</button></div>
      <details class="st-research-note"><summary>${esc(text("About this setup", "О сетапе", "Сетап туралы"))}</summary><p class="hint">${esc(text("Retrospective exploratory result, Apr–Sep 2026. Cost filter OFF. Five trades do not prove profitability. This badge never changes with your sliders.", "Ретроспективная диагностика, апрель–сентябрь 2026. Фильтр издержек ВЫКЛЮЧЕН. Пять сделок не доказывают прибыльность. Эта подпись не меняется от ползунков.", "Ретроспективті тәжірибе, 5 мәміле кірістілікті дәлелдемейді. Шығын сүзгісі өшірулі."))}</p></details>
      <div class="bt-presets">${presets}</div>
      ${B.editingSetup ? `<form id="stSetupForm" class="st-setup-form"><label for="stPresetName">${esc(text("Setup name", "Название сетапа", "Сетап атауы"))}</label><input class="bt-input" id="stPresetName" maxlength="40" required value="${esc(B.setupName)}" ${B.saving ? "disabled" : ""}>
      <p class="hint">${esc(text("Saves all current strategy parameters; dates and results are not saved.", "Сохраняет все текущие параметры стратегии; даты и результаты не сохраняются.", "Стратегия параметрлерін сақтайды; күндер мен нәтижелер сақталмайды."))}</p>
      <div class="bt-presets"><button type="submit" class="ghost sm" id="stSavePreset" ${B.busy || B.saving ? "disabled" : ""}>${esc(B.saving ? text("Saving…","Сохранение…","Сақталуда…") : text("Save","Сохранить","Сақтау"))}</button><button type="button" class="ghost sm" id="stCancelSetup" ${B.saving ? "disabled" : ""}>${esc(text("Cancel","Отмена","Болдырмау"))}</button></div><p class="hint" role="alert" id="stSetupError">${esc(B.setupError)}</p></form>` : ""}
      <div class="st-dates"><label>${esc(t("From"))}<input id="stFrom" type="date" min="2026-03-09" value="${B.from}" ${B.busy ? "disabled" : ""}></label><label>${esc(t("To"))}<input id="stTo" type="date" min="2026-03-09" value="${B.to}" ${B.busy ? "disabled" : ""}></label></div>
      <p class="hint">${esc(text("Levels: previous UTC day. First selected day is warmup only; trading starts the following day. Confirmation: completed 1m bars, New York hours. Up to 184 complete data days.", "Уровни: предыдущий UTC-день. Первый выбранный день — только прогрев уровней; сделки начинаются со следующего. Подтверждение: закрытые 1m-свечи, часы Нью-Йорка. До 184 дней данных.", "Деңгей: алдыңғы UTC күні. Бірінші күн — дайындық; мәмілелер келесі күннен басталады. Жабылған 1m шамдар, Нью-Йорк уақыты. 184 күнге дейін."))}</p>
      ${groups.map(g=>`<details class="bt-group" open><summary>${esc(t(g))}</summary>${B.info.specs.filter(s=>s.group===g).map(control).join("")}</details>`).join("")}
      </div><div class="bt-col" id="stResults">${results()}</div></div>`;
    body.querySelectorAll("[data-param]").forEach(el=>{el.oninput=()=>{B.p.values[el.dataset.param]=el.type==="checkbox" ? +el.checked : +el.value;if(el.nextElementSibling?.tagName==="OUTPUT")el.nextElementSibling.textContent=n(+el.value,2);changed();};});
    $("stResearch").disabled=B.busy || B.saving;$("stResearch").onclick=()=>{B.p=clone(B.info.defaults);changed();render();};
    body.querySelectorAll("[data-preset]").forEach(el=>{el.disabled=B.busy || B.saving;el.onclick=()=>{B.p=clone(B.info.presets[el.dataset.preset]);changed();render();};});
    $("stAddSetup").onclick=()=>{B.editingSetup=true;B.setupError="";render();$("stPresetName").focus();};
    if ($("stSetupForm")) {
      $("stPresetName").oninput=()=>{B.setupName=$("stPresetName").value;};
      $("stCancelSetup").onclick=()=>{B.editingSetup=false;B.setupName="";B.setupError="";render();};
      $("stSetupForm").onsubmit=async(e)=>{
        e.preventDefault();if(B.busy || B.saving)return;
        const name=B.setupName.trim();
        if(!name || Object.hasOwn(B.info.presets || {},name)) {
          B.setupError=text("Enter a new, unique setup name.","Введите новое уникальное название сетапа.","Жаңа бірегей сетап атауын енгізіңіз.");$("stSetupError").textContent=B.setupError;$("stPresetName").focus();return;
        }
        clearTimeout(saveTimer);const params=clone(B.p);B.saving=true;B.setupError="";render();
        try{await invoke("structural_save",{params,name});(B.info.presets ||= {})[name]=params;B.editingSetup=false;B.setupName="";}
        catch(error){B.setupError=String(error);log(B.setupError,"bad");}
        B.saving=false;if(S.panel==="structural")render();
      };
    }
    for(const [id,key] of [["stFrom","from"],["stTo","to"]])$(id).onchange=()=>{B[key]=$(id).value;B.stale=true;render();};
    $("stRun").onclick=run;
  }
  async function run() {
    if(B.busy || B.saving)return;clearTimeout(saveTimer);B.busy=true;B.error="";B.progress=t("Loading…");render();
    const params=clone(B.p);
    try{await invoke("structural_save",{params,name:null});B.result=await invoke("structural_backtest",{params,from:B.from,to:B.to});B.stale=false;}
    catch(e){B.error=String(e);log(B.error,"bad");}
    B.busy=false;B.progress="";if(S.panel==="structural")render();
  }
  window.__TAURI__?.event.listen("structural_progress",({payload:p})=>{B.progress=`${t("Loading…")} ${p.done}/${p.total}`;const el=$("stProgress");if(el)el.textContent=B.progress;});
  A.structural={render};
})();
