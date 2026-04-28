import './popup.css';

const DRAFT_STORAGE_KEY = 'cloud_popup_draft_v1';

type PopupDraft = {
  serverUrl: string;
  workspaceId: string;
  actorId: string;
  pairingCode: string;
  ttlSecs: number;
  probeSessionId: string;
  probeCmd: string;
  probePayload: string;
};

type CloudActorStatus = {
  supported: boolean;
  actor_mode: string;
  config_path: string;
  configured: boolean;
  connected: boolean;
  actor_id?: string;
  workspace_id?: string;
  server_url?: string;
  websocket_url?: string;
  paired_at_ms?: number;
  connect_timeout_ms?: number;
  request_timeout_ms?: number;
  last_connected_at_ms?: number;
  last_ready_at_ms?: number;
  last_error?: string;
};

type CloudUiState = {
  success: boolean;
  nativeHostConnected: boolean;
  nativeHostName?: string | null;
  cloudStatus?: CloudActorStatus | null;
  error?: string;
};

const defaultDraft: PopupDraft = {
  serverUrl: 'http://127.0.0.1:8787',
  workspaceId: 'default',
  actorId: 'local-dev-browser',
  pairingCode: '',
  ttlSecs: 600,
  probeSessionId: 'popup-remote-smoke',
  probeCmd: 'get_current_url',
  probePayload: '',
};

const app = document.querySelector<HTMLDivElement>('#app');
if (!app) {
  throw new Error('Popup root not found');
}

app.innerHTML = `
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
`;

const nativePill = document.querySelector<HTMLDivElement>('#native-pill')!;
const cloudPill = document.querySelector<HTMLDivElement>('#cloud-pill')!;
const serverUrlInput = document.querySelector<HTMLInputElement>('#server-url')!;
const workspaceIdInput = document.querySelector<HTMLInputElement>('#workspace-id')!;
const ttlSecsInput = document.querySelector<HTMLInputElement>('#ttl-secs')!;
const actorIdInput = document.querySelector<HTMLInputElement>('#actor-id')!;
const pairingCodeInput = document.querySelector<HTMLInputElement>('#pairing-code')!;
const probeSessionIdInput = document.querySelector<HTMLInputElement>('#probe-session-id')!;
const probeCmdInput = document.querySelector<HTMLInputElement>('#probe-cmd')!;
const probePayloadInput = document.querySelector<HTMLTextAreaElement>('#probe-payload')!;
const metricActor = document.querySelector<HTMLDivElement>('#metric-actor')!;
const metricWorkspace = document.querySelector<HTMLDivElement>('#metric-workspace')!;
const metricServer = document.querySelector<HTMLDivElement>('#metric-server')!;
const metricReady = document.querySelector<HTMLDivElement>('#metric-ready')!;
const statusLog = document.querySelector<HTMLDivElement>('#status-log')!;
const probeResult = document.querySelector<HTMLPreElement>('#probe-result')!;
const issueCodeButton = document.querySelector<HTMLButtonElement>('#issue-code')!;
const pairActorButton = document.querySelector<HTMLButtonElement>('#pair-actor')!;
const refreshStatusButton = document.querySelector<HTMLButtonElement>('#refresh-status')!;
const disconnectActorButton = document.querySelector<HTMLButtonElement>('#disconnect-actor')!;
const probeRunButton = document.querySelector<HTMLButtonElement>('#probe-run')!;
const presetButtons = Array.from(document.querySelectorAll<HTMLButtonElement>('[data-probe-cmd]'));

let currentDraft: PopupDraft = { ...defaultDraft };
let latestState: CloudUiState | null = null;

function formatTimestamp(timestamp?: number): string {
  if (!timestamp) return 'never';
  return new Date(timestamp).toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

function setLog(message: string, tone: 'neutral' | 'ok' | 'error' = 'neutral'): void {
  statusLog.textContent = message;
  statusLog.classList.remove('ok', 'error');
  if (tone === 'ok') statusLog.classList.add('ok');
  if (tone === 'error') statusLog.classList.add('error');
}

function setBusy(isBusy: boolean): void {
  for (const button of [issueCodeButton, pairActorButton, refreshStatusButton, disconnectActorButton, probeRunButton]) {
    button.disabled = isBusy;
  }
}

async function loadDraft(): Promise<PopupDraft> {
  const stored = await chrome.storage.local.get(DRAFT_STORAGE_KEY);
  return {
    ...defaultDraft,
    ...(stored?.[DRAFT_STORAGE_KEY] || {}),
  };
}

async function saveDraft(): Promise<void> {
  await chrome.storage.local.set({
    [DRAFT_STORAGE_KEY]: currentDraft,
  });
}

function readDraftFromInputs(): PopupDraft {
  return {
    serverUrl: serverUrlInput.value.trim(),
    workspaceId: workspaceIdInput.value.trim() || 'default',
    actorId: actorIdInput.value.trim(),
    pairingCode: pairingCodeInput.value.trim().toUpperCase(),
    ttlSecs: Math.max(60, Number(ttlSecsInput.value || defaultDraft.ttlSecs)),
    probeSessionId: probeSessionIdInput.value.trim() || defaultDraft.probeSessionId,
    probeCmd: probeCmdInput.value.trim() || defaultDraft.probeCmd,
    probePayload: probePayloadInput.value,
  };
}

function applyDraftToInputs(draft: PopupDraft): void {
  serverUrlInput.value = draft.serverUrl;
  workspaceIdInput.value = draft.workspaceId;
  actorIdInput.value = draft.actorId;
  pairingCodeInput.value = draft.pairingCode;
  ttlSecsInput.value = String(draft.ttlSecs);
  probeSessionIdInput.value = draft.probeSessionId;
  probeCmdInput.value = draft.probeCmd;
  probePayloadInput.value = draft.probePayload;
}

function seedDraftFromStatus(status?: CloudActorStatus | null): void {
  if (!status?.configured) return;
  if (!currentDraft.serverUrl && status.server_url) currentDraft.serverUrl = status.server_url;
  if (!currentDraft.actorId && status.actor_id) currentDraft.actorId = status.actor_id;
  if (!currentDraft.workspaceId && status.workspace_id) currentDraft.workspaceId = status.workspace_id;
}

function renderState(state: CloudUiState | null): void {
  latestState = state;
  const nativeOnline = !!state?.nativeHostConnected;
  const cloudOnline = !!state?.cloudStatus?.connected;
  nativePill.textContent = nativeOnline ? 'Native host online' : 'Native host offline';
  nativePill.className = `pill ${nativeOnline ? 'online' : 'offline'}`;
  cloudPill.textContent = cloudOnline ? 'Cloud actor live' : 'Cloud actor idle';
  cloudPill.className = `pill ${cloudOnline ? 'online' : 'offline'}`;

  metricActor.textContent = state?.cloudStatus?.actor_id || 'not paired';
  metricWorkspace.textContent = state?.cloudStatus?.workspace_id || '-';
  metricServer.textContent = state?.cloudStatus?.server_url || '-';
  metricReady.textContent = formatTimestamp(state?.cloudStatus?.last_ready_at_ms);

  if (state?.error) {
    setLog(state.error, 'error');
  } else if (!nativeOnline) {
    setLog('Native host is disconnected. The popup can save form state, but live pairing needs the host.', 'error');
  } else if (cloudOnline) {
    setLog('Cloud actor is connected and ready for remote commands.', 'ok');
  } else if (state?.cloudStatus?.configured) {
    setLog(
      state.cloudStatus.last_error
        ? `Config saved, but the cloud actor is reconnecting: ${state.cloudStatus.last_error}`
        : 'Config saved locally. Waiting for the cloud actor to connect.',
      state.cloudStatus.last_error ? 'error' : 'neutral'
    );
  } else {
    setLog('No cloud actor config is applied yet. Issue or paste a pairing code to begin.', 'neutral');
  }
}

async function sendBackgroundMessage<T>(cmd: string, payload: Record<string, any> = {}): Promise<T> {
  return await chrome.runtime.sendMessage({ cmd, payload });
}

async function refreshState(): Promise<void> {
  const response = await sendBackgroundMessage<CloudUiState>('cloud_ui_get_state');
  seedDraftFromStatus(response.cloudStatus);
  applyDraftToInputs(currentDraft);
  renderState(response);
}

for (const input of [serverUrlInput, workspaceIdInput, ttlSecsInput, actorIdInput, pairingCodeInput]) {
  input.addEventListener('input', async () => {
    currentDraft = readDraftFromInputs();
    await saveDraft();
  });
}

for (const input of [probeSessionIdInput, probeCmdInput, probePayloadInput]) {
  input.addEventListener('input', async () => {
    currentDraft = readDraftFromInputs();
    await saveDraft();
  });
}

for (const button of presetButtons) {
  button.addEventListener('click', async () => {
    probeCmdInput.value = button.dataset.probeCmd || defaultDraft.probeCmd;
    probePayloadInput.value = button.dataset.probePayload || '';
    currentDraft = readDraftFromInputs();
    await saveDraft();
  });
}

issueCodeButton.addEventListener('click', async () => {
  setBusy(true);
  try {
    currentDraft = readDraftFromInputs();
    await saveDraft();
    const response = await sendBackgroundMessage<any>('cloud_ui_issue_pairing_code', {
      serverUrl: currentDraft.serverUrl,
      workspaceId: currentDraft.workspaceId,
      ttlSecs: currentDraft.ttlSecs,
    });
    if (!response?.success) {
      throw new Error(response?.error || 'Failed to issue pairing code');
    }
    currentDraft.pairingCode = response.pairing.pairing_code || '';
    pairingCodeInput.value = currentDraft.pairingCode;
    await saveDraft();
    setLog(`Issued pairing code ${currentDraft.pairingCode}. It expires ${formatTimestamp(response.pairing.expires_at_ms)}.`, 'ok');
  } catch (error: any) {
    setLog(error?.message || String(error), 'error');
  } finally {
    setBusy(false);
  }
});

pairActorButton.addEventListener('click', async () => {
  setBusy(true);
  try {
    currentDraft = readDraftFromInputs();
    if (!currentDraft.serverUrl) {
      throw new Error('Server URL is required');
    }
    if (!currentDraft.actorId) {
      throw new Error('Actor ID is required');
    }
    if (!currentDraft.pairingCode) {
      throw new Error('Pairing code is required');
    }
    await saveDraft();
    const response = await sendBackgroundMessage<any>('cloud_ui_pair_actor', {
      serverUrl: currentDraft.serverUrl,
      actorId: currentDraft.actorId,
      pairingCode: currentDraft.pairingCode,
    });
    if (!response?.success) {
      throw new Error(response?.error || 'Failed to pair actor');
    }
    currentDraft.pairingCode = '';
    pairingCodeInput.value = '';
    await saveDraft();
    setLog(`Paired ${response.pairing?.actor_id || currentDraft.actorId} and applied local cloud config.`, 'ok');
    await refreshState();
  } catch (error: any) {
    setLog(error?.message || String(error), 'error');
  } finally {
    setBusy(false);
  }
});

refreshStatusButton.addEventListener('click', async () => {
  setBusy(true);
  try {
    await refreshState();
  } catch (error: any) {
    setLog(error?.message || String(error), 'error');
  } finally {
    setBusy(false);
  }
});

disconnectActorButton.addEventListener('click', async () => {
  setBusy(true);
  try {
    const response = await sendBackgroundMessage<any>('cloud_ui_disconnect_actor');
    if (!response?.success) {
      throw new Error(response?.error || 'Failed to disconnect actor');
    }
    setLog('Removed the local cloud actor config. The browser will stop listening after the current session closes.', 'ok');
    await refreshState();
  } catch (error: any) {
    setLog(error?.message || String(error), 'error');
  } finally {
    setBusy(false);
  }
});

probeRunButton.addEventListener('click', async () => {
  setBusy(true);
  probeResult.textContent = 'Running remote probe…';
  try {
    currentDraft = readDraftFromInputs();
    if (!currentDraft.serverUrl) {
      throw new Error('Server URL is required');
    }
    if (!currentDraft.actorId) {
      throw new Error('Actor ID is required');
    }
    if (!currentDraft.probeCmd) {
      throw new Error('Command is required');
    }
    await saveDraft();

    const response = await sendBackgroundMessage<any>('cloud_ui_run_remote_command', {
      serverUrl: currentDraft.serverUrl,
      actorId: currentDraft.actorId,
      sessionId: currentDraft.probeSessionId,
      cmd: currentDraft.probeCmd,
      commandPayload: currentDraft.probePayload,
    });
    if (!response?.success) {
      throw new Error(response?.error || 'Remote probe failed');
    }

    probeResult.textContent = JSON.stringify(response.result, null, 2);
    setLog(`Remote command ${currentDraft.probeCmd} completed.`, 'ok');
    await refreshState();
  } catch (error: any) {
    probeResult.textContent = error?.message || String(error);
    setLog(error?.message || String(error), 'error');
  } finally {
    setBusy(false);
  }
});

(async () => {
  currentDraft = await loadDraft();
  applyDraftToInputs(currentDraft);
  setBusy(true);
  try {
    await refreshState();
  } finally {
    setBusy(false);
  }
})();
