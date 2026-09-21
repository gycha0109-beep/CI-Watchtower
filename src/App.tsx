import { useCallback, useEffect, useMemo, useState, type FormEvent } from 'react';
import { api } from './api';
import type { Dashboard, DashboardTrack, MonitoredRepository, Settings, TrackInput, WorkflowRunSummary } from './types';

type TrackFormState = Omit<TrackInput, 'longCiMinutes'> & { longCiMinutes: number | '' };

const emptyTrack = (): TrackFormState => ({
  name: '',
  trackKey: '',
  longCiMinutes: '',
});

function formatDuration(seconds: number | null | undefined) {
  if (seconds == null) return '—';
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m < 60) return `${m}m ${s}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

function statusLabel(item: DashboardTrack): [string, string] {
  switch (item.health) {
    case 'green': return ['GREEN', 'status green'];
    case 'red': return ['RED', 'status red'];
    case 'running': return ['RUNNING', 'status running'];
    case 'queued': return ['QUEUED', 'status queued'];
    case 'completed_other': return ['DONE', 'status amber'];
    default: return ['WAITING', 'status muted'];
  }
}

function runDot(run: WorkflowRunSummary) {
  if (run.status === 'completed') return run.conclusion === 'success' ? 'green-dot' : 'red-dot';
  if (['queued', 'requested', 'pending', 'waiting'].includes(run.status)) return 'queued-dot';
  return 'running-dot';
}

function repositoryState(repo: MonitoredRepository) {
  if (!repo.enabled) return 'OFF';
  if (repo.lastError) return 'ERROR';
  if (repo.runningCount > 0) return `RUNNING ${repo.runningCount}`;
  if (repo.queuedCount > 0) return `QUEUED ${repo.queuedCount}`;
  return 'IDLE';
}

function App() {
  const [dashboard, setDashboard] = useState<Dashboard | null>(null);
  const [trackForm, setTrackForm] = useState<TrackFormState>(emptyTrack());
  const [repoInput, setRepoInput] = useState('');
  const [token, setToken] = useState('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

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
    if (!dashboard) return '—';
    return dashboard.congestionLevel.toUpperCase();
  }, [dashboard]);

  const act = async (work: () => Promise<unknown>, message?: string, poll = false) => {
    try {
      setError(null);
      setNotice(null);
      await work();
      if (message) setNotice(message);
      await refresh(poll);
    } catch (e) {
      setError(String(e));
    }
  };

  const submitTrack = async (e: FormEvent) => {
    e.preventDefault();
    const payload: TrackInput = {
      id: editingId ?? undefined,
      name: trackForm.name.trim(),
      trackKey: trackForm.trackKey.trim().toLowerCase(),
      longCiMinutes: Number(trackForm.longCiMinutes),
    };
    if (!payload.name || !payload.trackKey || payload.longCiMinutes <= 0) return;
    await act(async () => {
      await api.saveTrack(payload);
      setTrackForm(emptyTrack());
      setEditingId(null);
    }, '트랙을 저장했습니다.', true);
  };

  const submitRepository = async (e: FormEvent) => {
    e.preventDefault();
    const repo = repoInput.trim();
    if (!repo) return;
    await act(async () => {
      await api.saveRepository({ repo, enabled: true });
      setRepoInput('');
    }, '감시 저장소를 추가했습니다.', true);
  };

  const editTrack = (item: DashboardTrack) => {
    setEditingId(item.track.id);
    setTrackForm({
      id: item.track.id,
      name: item.track.name,
      trackKey: item.track.trackKey,
      longCiMinutes: item.track.longCiMinutes,
    });
    window.scrollTo({ top: 0, behavior: 'smooth' });
  };

  const saveToken = async (e: FormEvent) => {
    e.preventDefault();
    if (!token.trim()) return;
    await act(async () => {
      await api.setGithubToken(token.trim());
      setToken('');
    }, 'PAT 저장 및 재조회 검증을 완료했습니다.', true);
  };

  const updateSettings = async (patch: Partial<Settings>) => {
    if (!dashboard) return;
    const next = { ...dashboard.settings, ...patch };
    await act(() => api.saveSettings(next));
  };

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">GITHUB ACTIONS CONTROL · V0.2</p>
          <h1>CI Watchtower</h1>
          <p className="subtitle">Repository 전체 Run을 수집하고 Track Key로 자동 귀속합니다.</p>
        </div>
        <button className="primary" onClick={() => void refresh(true)} disabled={busy}>
          {busy ? '확인 중…' : '지금 확인'}
        </button>
      </header>

      {error && <div className="error-banner">{error}</div>}
      {notice && <div className="notice-banner">{notice}</div>}

      <section className="summary-grid five">
        <div className="summary-card"><span>Running</span><strong>{dashboard?.runningCount ?? 0}</strong></div>
        <div className="summary-card"><span>Queued</span><strong>{dashboard?.queuedCount ?? 0}</strong></div>
        <div className="summary-card"><span>Unassigned</span><strong>{dashboard?.unassignedCount ?? 0}</strong></div>
        <div className={`summary-card congestion ${dashboard?.congestionLevel ?? 'safe'}`}><span>Queue</span><strong>{congestionText}</strong></div>
        <div className="summary-card"><span>GitHub PAT</span><strong>{dashboard?.tokenConfigured ? 'READY' : 'MISSING'}</strong></div>
      </section>

      <div className="layout-grid">
        <section className="panel controls-panel">
          <h2>{editingId ? '트랙 수정' : '트랙 등록'}</h2>
          <form onSubmit={submitTrack} className="stack-form">
            <label>트랙 이름
              <input value={trackForm.name} onChange={e => setTrackForm({ ...trackForm, name: e.target.value })} placeholder="예: 프론트 연동" />
            </label>
            <label>Track Key
              <input value={trackForm.trackKey} onChange={e => setTrackForm({ ...trackForm, trackKey: e.target.value })} placeholder="frontend-integration" />
              <span className="hint">저장소와 대화 번호가 바뀌어도 같은 작업축이면 유지합니다.</span>
            </label>
            <label>장기 CI 기준시간
              <input type="number" min="1" step="1" value={trackForm.longCiMinutes} onChange={e => setTrackForm({ ...trackForm, longCiMinutes: e.target.value === '' ? '' : Number(e.target.value) })} placeholder="8" />
            </label>
            <div className="row-actions">
              <button className="primary" type="submit">{editingId ? '수정 저장' : '트랙 추가'}</button>
              {editingId && <button type="button" className="ghost" onClick={() => { setEditingId(null); setTrackForm(emptyTrack()); }}>취소</button>}
            </div>
          </form>

          <hr />
          <h2>감시 저장소</h2>
          <form onSubmit={submitRepository} className="stack-form">
            <label>Repository
              <input value={repoInput} onChange={e => setRepoInput(e.target.value)} placeholder="gycha0109-beep/MyeongHa" />
            </label>
            <button className="primary" type="submit">저장소 추가</button>
          </form>
          <div className="repo-list">
            {dashboard?.repositories.map(repo => (
              <div className="repo-item" key={repo.id}>
                <div>
                  <b>{repo.repo}</b>
                  <span className={repo.lastError ? 'repo-error' : ''}>{repositoryState(repo)}</span>
                </div>
                <div className="row-actions">
                  <button className="ghost small" onClick={() => void act(() => api.saveRepository({ id: repo.id, repo: repo.repo, enabled: !repo.enabled }), undefined, true)}>
                    {repo.enabled ? '중지' : '감시'}
                  </button>
                  <button className="danger-ghost small" onClick={() => void act(() => api.deleteRepository(repo.id))}>삭제</button>
                </div>
              </div>
            ))}
          </div>

          <hr />
          <h2>GitHub 인증</h2>
          <form onSubmit={saveToken} className="stack-form">
            <label>Fine-grained PAT
              <input type="password" value={token} onChange={e => setToken(e.target.value)} placeholder="github_pat_..." autoComplete="off" />
            </label>
            <p className="security-note">Windows Credential Manager에 저장하고 즉시 재조회하여 저장 성공 여부를 검증합니다.</p>
            <div className="row-actions">
              <button className="primary" type="submit">PAT 저장</button>
              {dashboard?.tokenConfigured && <button type="button" className="danger-ghost" onClick={() => void act(() => api.clearGithubToken())}>삭제</button>}
            </div>
          </form>

          <hr />
          <h2>감시 설정</h2>
          {dashboard && <div className="stack-form">
            <label>Queue 혼잡 경고 기준
              <input type="number" min="1" value={dashboard.settings.queueCongestionThreshold} onBlur={e => void updateSettings({ queueCongestionThreshold: Number(e.target.value) })} onChange={e => setDashboard({ ...dashboard, settings: { ...dashboard.settings, queueCongestionThreshold: Number(e.target.value) } })} />
            </label>
            <label>활성 Polling(초)
              <input type="number" min="10" value={dashboard.settings.activePollSeconds} onBlur={e => void updateSettings({ activePollSeconds: Number(e.target.value) })} onChange={e => setDashboard({ ...dashboard, settings: { ...dashboard.settings, activePollSeconds: Number(e.target.value) } })} />
            </label>
            <label>유휴 Polling(초)
              <input type="number" min="30" value={dashboard.settings.idlePollSeconds} onBlur={e => void updateSettings({ idlePollSeconds: Number(e.target.value) })} onChange={e => setDashboard({ ...dashboard, settings: { ...dashboard.settings, idlePollSeconds: Number(e.target.value) } })} />
            </label>
          </div>}
        </section>

        <section className="tracks-column">
          <section className="panel inbox-panel">
            <div className="section-head">
              <div>
                <p className="eyebrow">ATTRIBUTION INBOX</p>
                <h2>미귀속 CI</h2>
              </div>
              <strong>{dashboard?.unassignedCount ?? 0}</strong>
            </div>
            {(dashboard?.unassignedRuns.length ?? 0) === 0 ? (
              <p className="muted-copy">현재 확인이 필요한 미귀속 CI가 없습니다.</p>
            ) : dashboard?.unassignedRuns.map(run => (
              <div className="unassigned-run" key={run.id}>
                <div className="unassigned-main">
                  <span className={`dot ${runDot(run)}`} />
                  <div>
                    <b>{run.workflowName}</b>
                    <p>{run.repository} · {run.event} · {run.headBranch ?? 'detached'} · <span className="mono">{run.headSha.slice(0, 8)}</span></p>
                    <p className="reason">{run.resolutionStatus === 'conflict' ? `TRACK CONFLICT · ${run.attributionReason ?? '명시 신호 충돌'}` : (run.attributionReason ?? '명시적 Track Key를 찾지 못했습니다.')}</p>
                  </div>
                </div>
                <div className="assignment-actions">
                  {(dashboard?.tracks ?? []).map(item => (
                    <button key={item.track.id} className="ghost small" onClick={() => void act(() => api.assignRun(run.id, item.track.id), `${item.track.name}에 귀속했습니다.`)}>
                      {item.track.name}
                    </button>
                  ))}
                  <button className="danger-ghost small" onClick={() => void act(() => api.ignoreRun(run.id))}>무시</button>
                  <button className="ghost small" onClick={() => void api.openExternal(run.htmlUrl)}>Actions</button>
                </div>
              </div>
            ))}
          </section>

          {(dashboard?.tracks.length ?? 0) === 0 && (
            <div className="panel empty-state"><h2>등록된 트랙이 없습니다.</h2><p>Track Key와 장기 CI 기준만 등록하십시오.</p></div>
          )}

          {dashboard?.tracks.map(item => {
            const [label, cls] = statusLabel(item);
            return (
              <article className="panel track-card" key={item.track.id}>
                <div className="track-head">
                  <div>
                    <div className={cls}>{label}</div>
                    <h3>{item.track.name}</h3>
                    <p className="mono">{item.track.trackKey}</p>
                  </div>
                  <div className="track-actions">
                    <button className="icon-btn" onClick={() => editTrack(item)}>수정</button>
                    <button className="icon-btn danger" onClick={() => void act(() => api.deleteTrack(item.track.id))}>삭제</button>
                  </div>
                </div>

                <div className="metrics-row three">
                  <div><span>현재 경과</span><b>{formatDuration(item.elapsedSeconds)}</b></div>
                  <div><span>최근 20회 평균</span><b>{formatDuration(item.averageDurationSeconds)}</b></div>
                  <div><span>장기 기준</span><b>{item.track.longCiMinutes}m</b></div>
                </div>

                <div className="runs-list">
                  {item.runs.length === 0 ? <p className="muted-copy">아직 이 Track Key에 귀속된 Run이 없습니다.</p> : item.runs.map(run => (
                    <button className="run-row v2" key={run.id} onClick={() => void api.openExternal(run.htmlUrl)}>
                      <span className={`dot ${runDot(run)}`} />
                      <span className="run-main">
                        <b>{run.workflowName}</b>
                        <small>{run.repository} · {run.headBranch ?? 'detached'} · <span className="mono">{run.headSha.slice(0, 8)}</span></small>
                      </span>
                      <span className="run-state">{run.status}{run.conclusion ? ` · ${run.conclusion}` : ''}</span>
                      <span>{formatDuration(run.elapsedSeconds)}</span>
                      <span className="attribution">{run.attributionSource ?? 'resolver'} · {run.confidence ?? 0}</span>
                    </button>
                  ))}
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
