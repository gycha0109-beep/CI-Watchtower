import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import { api } from './api';
import type { Dashboard, DashboardTrack, Settings, TrackInput } from './types';

type TrackFormState = Omit<TrackInput, 'longCiMinutes'> & { longCiMinutes: number | '' };

const emptyTrack = (): TrackFormState => ({
  name: '',
  repo: '',
  sourceMode: 'branch',
  branch: '',
  prNumber: null,
  workflowFilter: '',
  longCiMinutes: '',
});

function formatDuration(seconds: number | null | undefined) {
  if (seconds == null) return '—';
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

function statusLabel(item: DashboardTrack) {
  switch (item.state.health) {
    case 'green': return ['GREEN', 'status green'];
    case 'red': return ['RED', 'status red'];
    case 'running': return ['RUNNING', 'status running'];
    case 'queued': return ['QUEUED', 'status queued'];
    case 'completed_other': return ['DONE', 'status amber'];
    case 'error': return ['ERROR', 'status red'];
    default: return ['WAITING', 'status muted'];
  }
}

function App() {
  const [dashboard, setDashboard] = useState<Dashboard | null>(null);
  const [trackForm, setTrackForm] = useState<TrackFormState>(emptyTrack());
  const [token, setToken] = useState('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async (poll = false) => {
    try {
      setBusy(true);
      setError(null);
      setDashboard(await (poll ? api.pollNow() : api.getDashboard()));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void refresh(false);
    const id = window.setInterval(() => void refresh(false), 5000);
    return () => window.clearInterval(id);
  }, [refresh]);

  const congestionText = useMemo(() => {
    if (!dashboard) return '';
    if (dashboard.congestionLevel === 'congested') return 'CONGESTED';
    if (dashboard.congestionLevel === 'busy') return 'BUSY';
    return 'SAFE';
  }, [dashboard]);

  const submitTrack = async (e: FormEvent) => {
    e.preventDefault();
    const payload: TrackInput = {
      ...trackForm,
      id: editingId ?? undefined,
      name: trackForm.name.trim(),
      repo: trackForm.repo.trim(),
      branch: trackForm.sourceMode === 'branch' ? trackForm.branch?.trim() || null : null,
      prNumber: trackForm.sourceMode === 'pr' ? Number(trackForm.prNumber) : null,
      workflowFilter: trackForm.workflowFilter?.trim() || null,
      longCiMinutes: Number(trackForm.longCiMinutes),
    };
    if (!payload.name || !payload.repo || !payload.longCiMinutes || payload.longCiMinutes <= 0) return;
    await api.saveTrack(payload);
    setTrackForm(emptyTrack());
    setEditingId(null);
    await refresh(true);
  };

  const editTrack = (item: DashboardTrack) => {
    const t = item.track;
    setEditingId(t.id);
    setTrackForm({
      id: t.id,
      name: t.name,
      repo: t.repo,
      sourceMode: t.sourceMode,
      branch: t.branch,
      prNumber: t.prNumber,
      workflowFilter: t.workflowFilter,
      longCiMinutes: t.longCiMinutes,
    });
    window.scrollTo({ top: 0, behavior: 'smooth' });
  };

  const saveToken = async (e: FormEvent) => {
    e.preventDefault();
    if (!token.trim()) return;
    await api.setGithubToken(token.trim());
    setToken('');
    await refresh(true);
  };

  const updateSettings = async (patch: Partial<Settings>) => {
    if (!dashboard) return;
    const next = { ...dashboard.settings, ...patch };
    await api.saveSettings(next);
    setDashboard({ ...dashboard, settings: next });
  };

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">GITHUB ACTIONS CONTROL</p>
          <h1>CI Watchtower</h1>
          <p className="subtitle">ChatGPT 트랙별 CI 완료·실패·혼잡 감시</p>
        </div>
        <button className="primary" onClick={() => void refresh(true)} disabled={busy}>
          {busy ? '확인 중…' : '지금 확인'}
        </button>
      </header>

      {error && <div className="error-banner">{error}</div>}

      <section className="summary-grid">
        <div className="summary-card"><span>Running</span><strong>{dashboard?.runningCount ?? 0}</strong></div>
        <div className="summary-card"><span>Queued</span><strong>{dashboard?.queuedCount ?? 0}</strong></div>
        <div className={`summary-card congestion ${dashboard?.congestionLevel ?? 'safe'}`}><span>Queue</span><strong>{congestionText || '—'}</strong></div>
        <div className="summary-card"><span>GitHub PAT</span><strong>{dashboard?.tokenConfigured ? 'READY' : 'MISSING'}</strong></div>
      </section>

      <div className="layout-grid">
        <section className="panel controls-panel">
          <h2>{editingId ? '트랙 수정' : '트랙 등록'}</h2>
          <form onSubmit={submitTrack} className="stack-form">
            <label>트랙 이름<input value={trackForm.name} onChange={e => setTrackForm({ ...trackForm, name: e.target.value })} placeholder="예: 관상 연구 2" /></label>
            <label>Repository<input value={trackForm.repo} onChange={e => setTrackForm({ ...trackForm, repo: e.target.value })} placeholder="gycha0109-beep/Saju" /></label>
            <div className="segmented">
              <button type="button" className={trackForm.sourceMode === 'branch' ? 'active' : ''} onClick={() => setTrackForm({ ...trackForm, sourceMode: 'branch' })}>Branch</button>
              <button type="button" className={trackForm.sourceMode === 'pr' ? 'active' : ''} onClick={() => setTrackForm({ ...trackForm, sourceMode: 'pr' })}>PR</button>
            </div>
            {trackForm.sourceMode === 'branch' ? (
              <label>Branch<input value={trackForm.branch ?? ''} onChange={e => setTrackForm({ ...trackForm, branch: e.target.value })} placeholder="feat/..." required /></label>
            ) : (
              <label>PR 번호<input type="number" min="1" value={trackForm.prNumber ?? ''} onChange={e => setTrackForm({ ...trackForm, prNumber: Number(e.target.value) })} required /></label>
            )}
            <label>Workflow 필터 <span className="hint">선택</span><input value={trackForm.workflowFilter ?? ''} onChange={e => setTrackForm({ ...trackForm, workflowFilter: e.target.value })} placeholder="예: CI, integration" /></label>
            <label>장기 CI 기준시간 <span className="hint">분 단위 직접 입력</span><input type="number" min="1" step="1" value={trackForm.longCiMinutes} onChange={e => setTrackForm({ ...trackForm, longCiMinutes: e.target.value === '' ? '' : Number(e.target.value) })} required /></label>
            <div className="row-actions">
              <button className="primary" type="submit">{editingId ? '수정 저장' : '트랙 추가'}</button>
              {editingId && <button type="button" className="ghost" onClick={() => { setEditingId(null); setTrackForm(emptyTrack()); }}>취소</button>}
            </div>
          </form>

          <hr />
          <h2>GitHub 인증</h2>
          <form onSubmit={saveToken} className="stack-form">
            <label>Fine-grained PAT<input type="password" value={token} onChange={e => setToken(e.target.value)} placeholder="github_pat_..." autoComplete="off" /></label>
            <p className="security-note">PAT은 앱 설정 파일이 아니라 OS 보안 저장소에 저장됩니다.</p>
            <div className="row-actions">
              <button className="primary" type="submit">PAT 저장</button>
              {dashboard?.tokenConfigured && <button type="button" className="danger-ghost" onClick={async () => { await api.clearGithubToken(); await refresh(false); }}>삭제</button>}
            </div>
          </form>

          <hr />
          <h2>감시 설정</h2>
          {dashboard && <div className="stack-form">
            <label>Queue 혼잡 경고 기준<input type="number" min="1" value={dashboard.settings.queueCongestionThreshold} onChange={e => void updateSettings({ queueCongestionThreshold: Number(e.target.value) })} /></label>
            <label>활성 트랙 Polling(초)<input type="number" min="10" value={dashboard.settings.activePollSeconds} onChange={e => void updateSettings({ activePollSeconds: Number(e.target.value) })} /></label>
            <label>유휴 Polling(초)<input type="number" min="30" value={dashboard.settings.idlePollSeconds} onChange={e => void updateSettings({ idlePollSeconds: Number(e.target.value) })} /></label>
            <label className="check-row"><input type="checkbox" checked={dashboard.settings.autoArchiveCompleted} onChange={e => void updateSettings({ autoArchiveCompleted: e.target.checked })} /> 완료 트랙 자동 정리</label>
            <button className="ghost" onClick={async () => { await api.unarchiveAll(); await refresh(false); }}>정리된 트랙 다시 표시</button>
          </div>}
        </section>

        <section className="tracks-column">
          {(dashboard?.tracks.length ?? 0) === 0 && <div className="panel empty-state"><h2>감시 중인 트랙이 없습니다.</h2><p>왼쪽에서 첫 트랙을 등록하십시오.</p></div>}
          {dashboard?.tracks.map(item => {
            const [label, cls] = statusLabel(item);
            return (
              <article className="panel track-card" key={item.track.id}>
                <div className="track-head">
                  <div>
                    <div className={cls}>{label}</div>
                    <h3>{item.track.name}</h3>
                    <p>{item.track.repo} · {item.track.sourceMode === 'pr' ? `PR #${item.track.prNumber}` : item.track.branch}</p>
                  </div>
                  <div className="track-actions">
                    <button className="icon-btn" onClick={() => editTrack(item)}>수정</button>
                    <button className="icon-btn danger" onClick={async () => { await api.deleteTrack(item.track.id); await refresh(false); }}>삭제</button>
                  </div>
                </div>

                <div className="metrics-row">
                  <div><span>현재 경과</span><b>{formatDuration(item.state.elapsedSeconds)}</b></div>
                  <div><span>최근 20회 평균</span><b>{formatDuration(item.state.averageDurationSeconds)}</b></div>
                  <div><span>장기 기준</span><b>{item.track.longCiMinutes}m</b></div>
                  <div><span>SHA</span><b className="mono">{item.state.headSha?.slice(0, 8) ?? '—'}</b></div>
                </div>

                {item.state.message && <div className="message-line">{item.state.message}</div>}

                <div className="runs-list">
                  {item.state.runs.length === 0 ? <p className="muted-copy">현재 SHA에서 확인된 workflow run이 없습니다.</p> : item.state.runs.map(run => (
                    <button className="run-row" key={run.id} onClick={() => void api.openExternal(run.htmlUrl)}>
                      <span className={`dot ${run.status === 'completed' ? (run.conclusion === 'success' ? 'green-dot' : 'red-dot') : run.status === 'queued' ? 'queued-dot' : 'running-dot'}`} />
                      <span className="run-name">{run.name}</span>
                      <span className="run-state">{run.status}{run.conclusion ? ` · ${run.conclusion}` : ''}</span>
                      <span>{formatDuration(run.elapsedSeconds)}</span>
                    </button>
                  ))}
                </div>

                <div className="card-footer">
                  {item.state.prUrl && <button className="ghost small" onClick={() => void api.openExternal(item.state.prUrl!)}>PR 열기</button>}
                  {item.state.latestRunUrl && <button className="ghost small" onClick={() => void api.openExternal(item.state.latestRunUrl!)}>Actions 열기</button>}
                </div>
              </article>
            );
          })}
        </section>
      </div>
    </main>
  );
}

export default App;
