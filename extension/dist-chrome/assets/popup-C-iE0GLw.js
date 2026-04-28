(function(){const r=document.createElement("link").relList;if(r&&r.supports&&r.supports("modulepreload"))return;for(const o of document.querySelectorAll('link[rel="modulepreload"]'))p(o);new MutationObserver(o=>{for(const a of o)if(a.type==="childList")for(const s of a.addedNodes)s.tagName==="LINK"&&s.rel==="modulepreload"&&p(s)}).observe(document,{childList:!0,subtree:!0});function u(o){const a={};return o.integrity&&(a.integrity=o.integrity),o.referrerPolicy&&(a.referrerPolicy=o.referrerPolicy),o.crossOrigin==="use-credentials"?a.credentials="include":o.crossOrigin==="anonymous"?a.credentials="omit":a.credentials="same-origin",a}function p(o){if(o.ep)return;o.ep=!0;const a=u(o);fetch(o.href,a)}})();const w="cloud_popup_draft_v1",l={serverUrl:"http://127.0.0.1:8787",workspaceId:"default",actorId:"local-dev-browser",pairingCode:"",ttlSecs:600,probeSessionId:"popup-remote-smoke",probeCmd:"get_current_url",probePayload:""},x=document.querySelector("#app");if(!x)throw new Error("Popup root not found");x.innerHTML=`
  <main class="shell">
    <section class="hero">
      <p class="eyebrow">Hosted Control Plane</p>
      <h1>Pair this browser, then leave it listening.</h1>
      <p class="hero-copy">
        The cloud decides what to run. This local actor keeps the real Chrome session, native bridge,
        and tab affinity alive on the machine.
      </p>
      <div class="hero-meta">
        <div id="native-pill" class="pill offline">Native host offline</div>
        <div id="cloud-pill" class="pill offline">Cloud actor idle</div>
      </div>
    </section>

    <section class="panel">
      <div class="panel-header">
        <div>
          <p class="panel-kicker">Connection</p>
          <h2 class="panel-title">Remote target</h2>
        </div>
      </div>
      <div class="form-grid">
        <label class="field">
          <span class="label">Server URL</span>
          <input id="server-url" class="input mono" type="url" placeholder="http://127.0.0.1:8787" />
        </label>
        <div class="field inline">
          <label class="field">
            <span class="label">Workspace</span>
            <input id="workspace-id" class="input mono" type="text" placeholder="default" />
          </label>
          <label class="field">
            <span class="label">TTL (sec)</span>
            <input id="ttl-secs" class="input mono" type="number" min="60" step="60" />
          </label>
        </div>
        <label class="field">
          <span class="label">Actor ID</span>
          <input id="actor-id" class="input mono" type="text" placeholder="local-dev-browser" />
        </label>
      </div>
    </section>

    <section class="panel">
      <div class="panel-header">
        <div>
          <p class="panel-kicker">Pairing</p>
          <h2 class="panel-title">Issue or redeem a code</h2>
        </div>
      </div>
      <div class="form-grid">
        <label class="field">
          <span class="label">Pairing code</span>
          <input id="pairing-code" class="input mono" type="text" placeholder="AB12CD34" />
        </label>
        <div class="button-row">
          <button id="issue-code" class="button secondary" type="button">Issue dev code</button>
          <button id="pair-actor" class="button primary" type="button">Pair & listen</button>
        </div>
      </div>
    </section>

    <section class="panel">
      <div class="panel-header">
        <div>
          <p class="panel-kicker">Runtime</p>
          <h2 class="panel-title">Local actor status</h2>
        </div>
        <div class="button-row">
          <button id="refresh-status" class="button secondary" type="button">Refresh</button>
          <button id="disconnect-actor" class="button ghost" type="button">Disconnect</button>
        </div>
      </div>
      <div class="status-card">
        <div class="status-grid">
          <div class="metric">
            <div class="metric-label">Actor</div>
            <div id="metric-actor" class="metric-value mono">not paired</div>
          </div>
          <div class="metric">
            <div class="metric-label">Workspace</div>
            <div id="metric-workspace" class="metric-value mono">-</div>
          </div>
          <div class="metric">
            <div class="metric-label">Server</div>
            <div id="metric-server" class="metric-value mono">-</div>
          </div>
          <div class="metric">
            <div class="metric-label">Last ready</div>
            <div id="metric-ready" class="metric-value">never</div>
          </div>
        </div>
        <div id="status-log" class="logline">Waiting for native host status…</div>
        <p class="footnote">
          Config is written on the local machine through the native host, so the actor can reconnect even when
          the MV3 worker gets suspended.
        </p>
      </div>
    </section>

    <section class="panel">
      <div class="panel-header">
        <div>
          <p class="panel-kicker">Probe</p>
          <h2 class="panel-title">Send a remote command</h2>
        </div>
      </div>
      <div class="preset-row">
        <button class="preset-button" type="button" data-probe-cmd="get_active_tab" data-probe-payload="">Active tab</button>
        <button class="preset-button" type="button" data-probe-cmd="get_current_url" data-probe-payload="">Current URL</button>
        <button class="preset-button" type="button" data-probe-cmd="get_dom_hash" data-probe-payload="">DOM hash</button>
      </div>
      <div class="form-grid">
        <div class="field inline inline-wide">
          <label class="field">
            <span class="label">Session</span>
            <input id="probe-session-id" class="input mono" type="text" placeholder="popup-remote-smoke" />
          </label>
          <label class="field">
            <span class="label">Command</span>
            <input id="probe-cmd" class="input mono" type="text" placeholder="get_current_url" />
          </label>
        </div>
        <label class="field">
          <span class="label">Payload JSON</span>
          <textarea id="probe-payload" class="input input-textarea mono" rows="5" placeholder='{"step":{"type":"get_current_url"}}'></textarea>
        </label>
        <div class="button-row">
          <button id="probe-run" class="button primary" type="button">Run probe</button>
        </div>
        <pre id="probe-result" class="result-card">No remote probe has been sent yet.</pre>
      </div>
    </section>
  </main>
`;const P=document.querySelector("#native-pill"),E=document.querySelector("#cloud-pill"),S=document.querySelector("#server-url"),_=document.querySelector("#workspace-id"),C=document.querySelector("#ttl-secs"),I=document.querySelector("#actor-id"),m=document.querySelector("#pairing-code"),k=document.querySelector("#probe-session-id"),y=document.querySelector("#probe-cmd"),h=document.querySelector("#probe-payload"),B=document.querySelector("#metric-actor"),F=document.querySelector("#metric-workspace"),M=document.querySelector("#metric-server"),$=document.querySelector("#metric-ready"),b=document.querySelector("#status-log"),g=document.querySelector("#probe-result"),U=document.querySelector("#issue-code"),R=document.querySelector("#pair-actor"),D=document.querySelector("#refresh-status"),A=document.querySelector("#disconnect-actor"),N=document.querySelector("#probe-run"),W=Array.from(document.querySelectorAll("[data-probe-cmd]"));let t={...l};function O(e){return e?new Date(e).toLocaleString([],{month:"short",day:"numeric",hour:"numeric",minute:"2-digit"}):"never"}function i(e,r="neutral"){b.textContent=e,b.classList.remove("ok","error"),r==="ok"&&b.classList.add("ok"),r==="error"&&b.classList.add("error")}function n(e){for(const r of[U,R,D,A,N])r.disabled=e}async function H(){const e=await chrome.storage.local.get(w);return{...l,...(e==null?void 0:e[w])||{}}}async function c(){await chrome.storage.local.set({[w]:t})}function d(){return{serverUrl:S.value.trim(),workspaceId:_.value.trim()||"default",actorId:I.value.trim(),pairingCode:m.value.trim().toUpperCase(),ttlSecs:Math.max(60,Number(C.value||l.ttlSecs)),probeSessionId:k.value.trim()||l.probeSessionId,probeCmd:y.value.trim()||l.probeCmd,probePayload:h.value}}function T(e){S.value=e.serverUrl,_.value=e.workspaceId,I.value=e.actorId,m.value=e.pairingCode,C.value=String(e.ttlSecs),k.value=e.probeSessionId,y.value=e.probeCmd,h.value=e.probePayload}function J(e){e!=null&&e.configured&&(!t.serverUrl&&e.server_url&&(t.serverUrl=e.server_url),!t.actorId&&e.actor_id&&(t.actorId=e.actor_id),!t.workspaceId&&e.workspace_id&&(t.workspaceId=e.workspace_id))}function K(e){var p,o,a,s,q,L;const r=!!(e!=null&&e.nativeHostConnected),u=!!((p=e==null?void 0:e.cloudStatus)!=null&&p.connected);P.textContent=r?"Native host online":"Native host offline",P.className=`pill ${r?"online":"offline"}`,E.textContent=u?"Cloud actor live":"Cloud actor idle",E.className=`pill ${u?"online":"offline"}`,B.textContent=((o=e==null?void 0:e.cloudStatus)==null?void 0:o.actor_id)||"not paired",F.textContent=((a=e==null?void 0:e.cloudStatus)==null?void 0:a.workspace_id)||"-",M.textContent=((s=e==null?void 0:e.cloudStatus)==null?void 0:s.server_url)||"-",$.textContent=O((q=e==null?void 0:e.cloudStatus)==null?void 0:q.last_ready_at_ms),e!=null&&e.error?i(e.error,"error"):r?u?i("Cloud actor is connected and ready for remote commands.","ok"):(L=e==null?void 0:e.cloudStatus)!=null&&L.configured?i(e.cloudStatus.last_error?`Config saved, but the cloud actor is reconnecting: ${e.cloudStatus.last_error}`:"Config saved locally. Waiting for the cloud actor to connect.",e.cloudStatus.last_error?"error":"neutral"):i("No cloud actor config is applied yet. Issue or paste a pairing code to begin.","neutral"):i("Native host is disconnected. The popup can save form state, but live pairing needs the host.","error")}async function v(e,r={}){return await chrome.runtime.sendMessage({cmd:e,payload:r})}async function f(){const e=await v("cloud_ui_get_state");J(e.cloudStatus),T(t),K(e)}for(const e of[S,_,C,I,m])e.addEventListener("input",async()=>{t=d(),await c()});for(const e of[k,y,h])e.addEventListener("input",async()=>{t=d(),await c()});for(const e of W)e.addEventListener("click",async()=>{y.value=e.dataset.probeCmd||l.probeCmd,h.value=e.dataset.probePayload||"",t=d(),await c()});U.addEventListener("click",async()=>{n(!0);try{t=d(),await c();const e=await v("cloud_ui_issue_pairing_code",{serverUrl:t.serverUrl,workspaceId:t.workspaceId,ttlSecs:t.ttlSecs});if(!(e!=null&&e.success))throw new Error((e==null?void 0:e.error)||"Failed to issue pairing code");t.pairingCode=e.pairing.pairing_code||"",m.value=t.pairingCode,await c(),i(`Issued pairing code ${t.pairingCode}. It expires ${O(e.pairing.expires_at_ms)}.`,"ok")}catch(e){i((e==null?void 0:e.message)||String(e),"error")}finally{n(!1)}});R.addEventListener("click",async()=>{var e;n(!0);try{if(t=d(),!t.serverUrl)throw new Error("Server URL is required");if(!t.actorId)throw new Error("Actor ID is required");if(!t.pairingCode)throw new Error("Pairing code is required");await c();const r=await v("cloud_ui_pair_actor",{serverUrl:t.serverUrl,actorId:t.actorId,pairingCode:t.pairingCode});if(!(r!=null&&r.success))throw new Error((r==null?void 0:r.error)||"Failed to pair actor");t.pairingCode="",m.value="",await c(),i(`Paired ${((e=r.pairing)==null?void 0:e.actor_id)||t.actorId} and applied local cloud config.`,"ok"),await f()}catch(r){i((r==null?void 0:r.message)||String(r),"error")}finally{n(!1)}});D.addEventListener("click",async()=>{n(!0);try{await f()}catch(e){i((e==null?void 0:e.message)||String(e),"error")}finally{n(!1)}});A.addEventListener("click",async()=>{n(!0);try{const e=await v("cloud_ui_disconnect_actor");if(!(e!=null&&e.success))throw new Error((e==null?void 0:e.error)||"Failed to disconnect actor");i("Removed the local cloud actor config. The browser will stop listening after the current session closes.","ok"),await f()}catch(e){i((e==null?void 0:e.message)||String(e),"error")}finally{n(!1)}});N.addEventListener("click",async()=>{n(!0),g.textContent="Running remote probe…";try{if(t=d(),!t.serverUrl)throw new Error("Server URL is required");if(!t.actorId)throw new Error("Actor ID is required");if(!t.probeCmd)throw new Error("Command is required");await c();const e=await v("cloud_ui_run_remote_command",{serverUrl:t.serverUrl,actorId:t.actorId,sessionId:t.probeSessionId,cmd:t.probeCmd,commandPayload:t.probePayload});if(!(e!=null&&e.success))throw new Error((e==null?void 0:e.error)||"Remote probe failed");g.textContent=JSON.stringify(e.result,null,2),i(`Remote command ${t.probeCmd} completed.`,"ok"),await f()}catch(e){g.textContent=(e==null?void 0:e.message)||String(e),i((e==null?void 0:e.message)||String(e),"error")}finally{n(!1)}});(async()=>{t=await H(),T(t),n(!0);try{await f()}finally{n(!1)}})();
